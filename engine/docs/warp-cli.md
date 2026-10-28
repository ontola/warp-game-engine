# The warp executable: games in Luau

`warp` runs a game made only of Luau scripts, sound patches and assets.
There is no Rust to write and nothing to recompile: edit a file while the
game runs and it reloads. This is the second way to make a game on Warp,
next to a Rust crate (see [making a game](making-a-game.md)). Sideways and
Growing Guns stay crates. A `warp` game suits quick ideas and live
iteration with an AI over MCP.

The crate is `engine/crates/warp`. The example is
[games/hello-warp](../../games/hello-warp/README.md), the starter that
`warp new` writes.

```sh
cargo build --release -p warp          # target/release/warp
warp new my-game                       # a starter game
warp run my-game                       # play it in a window
warp run my-game --live                # ... and let Claude change it over MCP
```

## Commands

| Command | What it does |
| --- | --- |
| `warp run DIR [--players N] [--live [PORT]] [--sky SKY] [--seed N] [--size WxH] [--fullscreen] [--frames N]` | Play in a window, with sound. `--live` serves MCP ([live control](live-control.md); all its `--live-*` flags work). `--frames N` quits after N frames (a smoke test). |
| `warp script DIR FILE.json --out OUT [--players N] [--sky SKY] [--seed N]` | A headless playtest in the [`app::script` format](building-and-running.md): ticks with controls, `cmd` lines, `expect` and `until` checks, `capture` (PNG), `state` (JSON) and `audio` (WAV). Exits with status 1 when a check fails. |
| `warp serve DIR [--live [PORT]] [--seconds T] [--size WxH]` | Headless in real time with live control on (port 7878 by default). Renders a frame only for a screenshot. Sound plays into a mixer that is never heard. |
| `warp check DIR [--ticks N]` | Load, run `init` and N idle ticks (default 120) without a GPU, then print the state. Exits with status 1 on any script or patch error. The fastest check of an edit. |
| `warp audio render FILE.luau [...]`, `check FILES`, `units` | The [`warp-audio`](audio.md#warp-audio) tool: render a patch and print its levels and a text spectrogram, check patches, list units. |
| `warp new DIR [--title TITLE]` | Write the starter game into a new folder. |

`SKY` is `gradient`, or `TIME[:WEATHER]` with TIME `sunrise`, `morning`,
`noon`, `afternoon`, `sunset` or `night` and WEATHER `clear`, `haze` or
`mist` ([live sky](live-sky-controls.md)).

Headless commands use any GPU adapter; `KERN_ADAPTER=software` runs them
on a CPU rasteriser on machines without a GPU. None of them opens a
window.

## A game folder

```text
my-game/
  game.toml           optional settings
  main.luau           the game
  sounds/*.luau       synth patches: sounds/pickup.luau is the sound "pickup"
  assets/             glTF models (.glb, .gltf)
  scripts/*.json      headless playtests (not modules)
  types.d.luau        the API, for reading and editor completion
```

`game.toml` (every key is optional; unknown keys are errors):

| Key | Default | Meaning |
| --- | --- | --- |
| `title` | the folder name | The window title. |
| `tick_rate` | 60 | Ticks per second; `tick(dt, input)` gets `dt = 1 / tick_rate`. |
| `players` | 1 | Local players, 1 to 4, in split screen. `--players` overrides it. |
| `seed` | 1 | Seed of the `random` generator. `--seed` overrides it. |
| `sky` | `gradient` | The sky at start. `--sky` overrides it; scripts call `sky.set`. |
| `max_entities` | 1024 | Entities alive at once (render slots reserved up front). |
| `size` | `[1280, 720]` | Window size in pixels. |
| `steps` | 200000 | Luau step budget per hook call (see [scripting](scripting.md#limits)). |
| `memory_mb` | 64 | Luau memory cap. |

Every `*.luau` file outside `sounds/`, `assets/` and `scripts/` is a
module (`*.d.luau` files are skipped). Most games need only `main.luau`.
Modules run their hooks in path order. There is no `require`, so modules
share only the world.

## The Luau API

The complete reference is
[`types.d.luau`](../crates/warp/starter/types.d.luau), the file `warp new`
writes into every game. The scripts run in the [script crate's
sandbox](scripting.md#the-sandbox): no clock, no files, no `math.random`,
a step budget and a memory cap.

Units are metres, seconds and radians (the camera's `fov` is in degrees).
+Y is up and a yaw of 0 faces -Z. Colours are sRGB from 0 to 1, given as
`"#rrggbb"`, `"#rrggbbaa"`, a vector, `{r, g, b, a}` or a grey number.
Options tables reject keys they do not know, so a typo is an error.

### Lifecycle

A module returns a table of hooks. All of them are optional.

| Hook | When |
| --- | --- |
| `init()` | Once, when the module first loads, and again after `restart`. |
| `tick(dt, input)` | Every tick, at `tick_rate`. `input[1]` is player 1's input. |
| `draw()` | Every displayed frame. For the HUD (and cameras). |
| `reload()` | After a saved edit has reloaded the module. |

`input[p]` holds `move_x` and `move_y` (left stick or WASD, -1 to 1,
y forward), `look_x` and `look_y` (radians this tick, mouse or right stick,
right and up positive), and `held` and `pressed` tables of buttons:
`south`, `east`, `west`, `north`, `left_shoulder`, `right_shoulder`,
`left_trigger`, `right_trigger`, `select`, `start`, `up`, `down`, `left`,
`right`, with the aliases `jump` (`south`: Space, gamepad A), `fire`
(`right_trigger`: left mouse button) and `aim` (`left_trigger`). The
keyboard mapping is in `app::input`.

### Functions

| Table | Functions |
| --- | --- |
| `world` | `spawn(shape, opts) -> id`, `set(id, opts)`, `get(id) -> entity or nil`, `exists(id)`, `remove(id)`, `clear()`, `find(tag) -> ids`, `count()`, `raycast(origin, dir, max?, query?) -> hit or nil`, `overlap(center, radius, query?) -> ids`, `move(id, vel, dt, query?) -> vel, grounded` |
| `camera` | `set(player, {pos, target, fov})`, `get(player)` |
| `sound` | `play(name, {at, db, pitch, <param> = value}) -> handle`, `set_param(handle, param, value)`, `stop(handle)`, `list()` |
| `hud` | `text(text, x, y, {size, color, align, width})`, `box(x, y, w, h, color)`, `size() -> w, h`, `view(player) -> x, y, w, h` |
| `sky` | `set(name)`, `get()` |
| `light` | `set({sun, sun_lux, sky_lux, exposure, fog_start, fog_end, fog_density})`, `reset()` |
| `random` | `float()`, `range(lo, hi)`, `int(lo, hi)`, `chance(p)`, `seed(n)` |
| `game` | `tick()`, `time()`, `dt()`, `players()`, `title()` |

- **Entities**: `world.spawn` takes a shape (`"cube"`, `"sphere"`,
  `"capsule"`, or a glTF path inside the game folder such as
  `"assets/crate.glb"`) and options: `pos`, `rot` (pitch, yaw, roll),
  `size` (a number or a vector; 1 fills a 1 m box, and the capsule is 2 m
  tall), `color`, `emissive` (0 off, 1 a clear glow in daylight),
  `roughness`, `metal`, `solid` (default true), `visible` and `tag`.
  `world.set` takes the same options plus `shape`.
- **Collision** is boxes. Each entity's box follows its position, size and
  yaw (pitch and roll are ignored). `raycast` hits solid boxes and returns
  `{id, distance, point, normal}`. `overlap` finds every box touching a
  sphere, solid or not, which is how pickups work. `move` slides an entity,
  as an upright cylinder the width and height of its box, along solid
  boxes (`kern::collide::Mover`), and returns the
  velocity after collisions and whether it stands on something. A
  `query` is `{tag, ignore = id or {ids}, all}`.
- **Sound**: `sound.play("pickup", { at = pos, pitch = 1.2, bright = 2 })`
  plays `sounds/pickup.luau` at a world position (no `at`: 2D), with the
  patch's `bright` parameter. A parameter the patch does not have is an
  error. The listeners are the cameras.
- **HUD** coordinates are pixels from the top left of the output. HUD
  calls belong in `draw`, because the HUD is cleared before every frame.
  Script and patch errors show in red at the bottom of the screen until
  they are fixed.
- **Sky and light**: `sky.set("sunset:haze")` bakes a procedural sky
  (about 0.2 s, on the next frame) and sets its sun, skylight, fog and
  exposure. `light.set` overrides parts of that.
- **Random**: SplitMix64 from the game's seed. `restart` reseeds it.

### State

Every module has a global `state` table that survives reloads. **main.luau's
`state` is the game's state**: `state()` reports it as JSON under
`/state`. Scripts check it with `expect` (`/state/score`), and MCP
clients read it with `state` and `query`. Numbers, strings, booleans,
vectors (`[x, y, z]`) and tables become JSON. Functions are left out.

The engine adds its own fields next to it: `tick`, `time`, `players`,
`sky`, `entities`, `tags` (entities per tag), `cameras`, `sounds`
(`loaded`, `played`, `recent`), `modules` and `errors`.

### Determinism

The same seed and the same inputs give the same game. The tick is fixed,
the random generator is seeded and entities are kept in id order. Rule:
everything that changes the game (`world.spawn`, `set`, `remove`,
`move`, `clear`, `random.*`, `sound.play`) is refused inside `draw()`,
because `draw` runs a varying number of times per tick. In a headless
script it runs only for a captured frame. Cameras, the HUD, the sky and the
light may be set anywhere. Set cameras in `tick` when sound positions
matter, because the listeners follow them.

### Costs

Entities live in a block of render slots reserved at start, so spawning,
moving and removing are cheap. A shape and look (colour, emissive,
roughness, metal) seen for the first time adds a mesh and a material.
That rebuilds the renderer on the next frame: free during `init`, a stall
of a second or so later (longer on the software rasteriser). Reuse looks
for things that come and go, as the starter's orbs do.

## Hot reload

The game checks its files about four times a second (every 15 ticks) and
right away on the `reload` command or tool.

- **main.luau** (or any module): an edited file loads again. Its `state`
  table and every entity stay, so the game keeps playing with the new
  code. `init` does not run again; `reload()` runs instead, if the module
  exports it. To start over with the new code, use `restart`, which
  clears the world and every `state`, reseeds the random generator and runs
  `init`.
- A load error (a syntax error, a module that returns something other than
  a table) keeps the previous version running and reports the file and
  line. A runtime error in a hook turns that module's hooks off until the
  file changes again. Both appear in `/errors`, in the log and on screen.
- **sounds/*.luau**: an edited patch is compiled and used by every later
  play. A broken patch keeps the last good version. `set_param` (MCP)
  changes a parameter's default until the file is saved again.
- `game.toml` and new assets are read at start.

## Live control over MCP

`warp run DIR --live` or `warp serve DIR` serves the
[live crate's](live-control.md) MCP endpoint. The game's name for the token
file and for `warp-mcp --game NAME` is the folder name (`hello-warp`).

Generic tools: `describe`, `state`, `query`, `screenshot`, `run_luau`
(Luau against the live game; `state` there is main.luau's), `list_files`,
`read_file` and `write_file` on the folder `game` (`.luau`, `.toml`, `.md`
and `.json` files; a write reloads at once and the reply says whether the
file loaded), `logs`, `play_sound`, `set_param` and `render_sound`.

Warp's own tools:

| Tool | What it does |
| --- | --- |
| `reload` | Reload edited modules and patches now; returns what changed and any errors. |
| `restart` | Start over: clear the world and every `state`, reseed, run `init`. |
| `command` | One console line: `lua CODE`, `reload`, `restart`, `sky NAME`, `sounds`, `entities [TAG]`, `logs`, `help`. |

A session with Claude Code:

```sh
warp serve games/hello-warp --live-token "$(openssl rand -hex 24)"   # or warp run ... --live
claude mcp add --transport stdio warp -- target/release/warp-mcp --game hello-warp
```

Then ask, for example, "make the orbs float up and down and give the
pickup a lower chime". The model reads `main.luau`, writes a new version,
checks `state.errors`, plays and renders the patch, and takes a screenshot.

The same commands work in scripts, as `cmd` steps:
`{ "cmd": "lua world.spawn('sphere', { pos = vector.create(0, 1, -3) })" }`.

## Tests

`cargo test -p warp` covers:

- unit tests: `game.toml`, sky names, Luau-to-JSON, the world (spawning,
  mesh reuse, rays, overlaps, the mover) and colour parsing;
- `tests/game.rs`, without a GPU: the starter equals games/hello-warp, `warp
  new` writes a game that plays and jumps, the same seed replays exactly,
  the API (rays, overlaps, typos, seeded numbers, the console), hot reload
  (state kept, `reload()` instead of `init`, a broken edit keeping the last
  version, runtime errors, restart) and patch reload and playback;
- `tests/headless.rs`, as child processes on `KERN_ADAPTER=software` (no
  window): `warp script games/hello-warp scripts/smoke.json` (the player
  runs into an orb, scores 1, jumps; the test reads the PNG and the WAV)
  and `warp serve --live 0` driven over MCP (the tool list, `run_luau`, a
  `write_file` edit that keeps the score, a broken edit, the sound tools, a
  screenshot, `restart`, `command`).

By hand, without a display:

```sh
KERN_ADAPTER=software warp script games/hello-warp games/hello-warp/scripts/smoke.json --out /tmp/hello
```

## Not yet

- Skinned and animated glTF, particles, local lights, physics bodies
  (the `physics` crate) and terrain are not exposed to Luau yet.
- No `require`: modules cannot share code.
- No online play. The determinism rules are meant to make lockstep or
  host-authoritative play possible later.
- `game.toml` and assets do not hot-reload.
- No interpolation between ticks: entities move at the tick rate.
