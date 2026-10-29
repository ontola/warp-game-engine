# The warp executable: games in Luau

`warp` runs a game made only of Luau scripts, sound patches and assets.
There is no Rust to write and nothing to recompile: edit a file while the
game runs and it reloads. This is the second way to make a game on Warp,
next to a Rust crate (see [making a game](making-a-game.md)). Sideways and
Growing Guns stay crates. A `warp` game suits quick ideas and live
iteration with an AI over MCP.

The crate is `engine/crates/warp`. The example is
[games/hello-warp](../../games/hello-warp/README.md), the starter that
`warp new` writes. [games/neon-trails](../../games/neon-trails/README.md),
[games/neon-siege](../../games/neon-siege/README.md) and
[games/ricochet-club](../../games/ricochet-club/README.md) are LÖVE party
games ported to Luau, with one shared view; Neon Siege is a co-op
twin-stick shooter with hundreds of shots, bursts and rings at once.
[games/bumper-royale](../../games/bumper-royale/README.md) (pucks on a
shrinking ring) and [games/paint-rush](../../games/paint-rush/README.md)
(592 floor tiles repainted live) are two more.

```sh
cargo build --release -p warp          # target/release/warp
warp new my-game                       # a starter game
warp run my-game                       # play it in a window
warp run my-game --live                # ... and let Claude change it over MCP
```

## Commands

| Command | What it does |
| --- | --- |
| `warp run DIR [--players N] [--bots N] [--live [PORT]] [--sky SKY] [--seed N] [--size WxH] [--fullscreen] [--frames N]` | Play in a window, with sound. `--live` serves MCP ([live control](live-control.md); all its `--live-*` flags work). `--frames N` quits after N frames (a smoke test). Launched by a GameNight party host, the host's seats decide the players (see [GameNight](#gamenight)). |
| `warp DIR` | The same as `warp run DIR`. This is how a GameNight host starts an installed game: the runtime gets the game folder. |
| `warp script DIR FILE.json --out OUT [--players N] [--sky SKY] [--seed N]` | A headless playtest in the [`app::script` format](building-and-running.md): ticks with controls, `cmd` lines, `expect` and `until` checks, `capture` (PNG), `state` (JSON) and `audio` (WAV). Exits with status 1 when a check fails. |
| `warp serve DIR [--live [PORT]] [--seconds T] [--size WxH]` | Headless in real time with live control on (port 7878 by default). Renders a frame only for a screenshot. Sound plays into a mixer that is never heard. |
| `warp check DIR [--ticks N]` | Load, run `init` and N idle ticks (default 120) without a GPU, then print the state. Exits with status 1 on any script or patch error. The fastest check of an edit. |
| `warp audio render FILE.luau [...]`, `check FILES`, `units` | The [`warp-audio`](audio.md#warp-audio) tool: render a patch and print its levels and a text spectrogram, check patches, list units. |
| `warp new DIR [--title TITLE]` | Write the starter game into a new folder; `warp new GAME.luau` writes it as [one file](#a-game-in-one-file). |

`DIR` can also be a [one-file game](#a-game-in-one-file) (`GAME.luau`).

`SKY` is `gradient`, or `TIME[:WEATHER]` with TIME `sunrise`, `morning`,
`noon`, `afternoon`, `sunset` or `night` and WEATHER `clear`, `haze` or
`mist` ([live sky](live-sky-controls.md)).

Headless commands use any GPU adapter; `KERN_ADAPTER=software` runs them
on a CPU rasteriser on machines without a GPU. None of them opens a
window.

## A game in one file

A whole game can be one `.luau` file. Its settings are the `--!` lines of
its leading comment block (the `game.toml` keys below), and its sound
patches are source strings in the module's exported `sounds` table:

```lua
-- Orb Hunt: collect the orbs.
--! title = "Orb Hunt"
--! sky = "sunset:haze"
--! bots = 2

local SOUNDS = {
	ping = [==[
length(0.3)
return expr("sin(tau * 880 * t) * exp(-t * 12) * 0.4")
]==],
}

local function init() ... end
local function tick(dt, input) ... sound.play("ping") ... end

return { sounds = SOUNDS, init = init, tick = tick }
```

```sh
warp new orb-hunt.luau      # the starter game in one file
warp run orb-hunt.luau      # or just: warp orb-hunt.luau
warp check orb-hunt.luau
```

Every command that takes a game folder takes such a file, or a folder whose only
`.luau` script (type files aside) is the game, which is how a packaged one-file
game runs. Only that file
loads (other `.luau` files beside it are not part of the game); glTF paths
and a `sounds/` folder, if there is one, are read relative to its folder,
and with `--live` that folder is what the AI can edit. Saving the file
reloads the code and any changed inline patch. Any module may export a
`sounds` table, in a folder game too.

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
| `id` | the folder name | The GameNight game id (lowercase letters, digits, dashes). A host's `GAMENIGHT_GAME_ID` wins. |
| `players` | 1 | People without a party host, 1 to 4, in split screen. `--players` overrides it. |
| `split` | true | One split-screen view per person; false gives one shared view (every `camera.set` moves that camera), as in an arena game. |
| `bots` | 0 | Bots without a party host, 0 to 7. `--bots` overrides it. |
| `min_players` | 1 | Bots fill up to this many players, with or without a party host (a duel needs two). 1 to 8. |
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
| `player_joined(p)` | A GameNight seat was added mid-game (`p` is a player table). |
| `player_left(p)` | A player left mid-game. |
| `players_changed()` | Names, colours or seats changed, after any join or leave too. |

`input[id]` is the input of the player with that id (see
[players](#players)); a bot's input is always neutral. It holds `move_x` and `move_y` (left stick or WASD, -1 to 1,
y forward), `look_x` and `look_y` (radians this tick, mouse or right stick,
right and up positive), `aim_x` and `aim_y` (where the right stick points,
-1 to 1, up positive, 0 on keyboard and mouse: twin-stick aiming), and
`held` and `pressed` tables of buttons:
`south`, `east`, `west`, `north`, `left_shoulder`, `right_shoulder`,
`left_trigger`, `right_trigger`, `select`, `start`, `up`, `down`, `left`,
`right`, with the aliases `jump` (`south`: Space, gamepad A), `fire`
(`right_trigger`: left mouse button) and `aim` (`left_trigger`). The
keyboard mapping is in `app::input`.

### Functions

| Table | Functions |
| --- | --- |
| `world` | `spawn(shape, opts) -> id`, `set(id, opts)`, `place(id, pos, yaw?)`, `warm(shape, look)`, `get(id) -> entity or nil`, `exists(id)`, `remove(id)`, `clear()`, `find(tag) -> ids`, `count()`, `raycast(origin, dir, max?, query?) -> hit or nil`, `overlap(center, radius, query?) -> ids`, `move(id, vel, dt, query?) -> vel, grounded` |
| `fx` | `burst(pos, {color, emissive, shape, count, speed, life, size, stretch, gravity, flat})`, `ring(pos, {color, emissive, radius, life, width, segments})`, `count() -> particles, rings` |
| `players` | `list() -> players`, `get(id) -> player or nil`, `count()`, `rumble(id, strength, seconds?)` |
| `camera` | `set(player, {pos, target, fov})`, `get(player)` |
| `sound` | `play(name, {at, db, pitch, <param> = value}) -> handle`, `set_param(handle, param, value)`, `stop(handle)`, `list()` |
| `hud` | `text(text, x, y, {size, color, align, width})`, `box(x, y, w, h, color)`, `size() -> w, h`, `view(player) -> x, y, w, h`, `project(pos, player?) -> x, y or nil` |
| `post` | `set({bloom, bloom_radius, bloom_threshold, contrast, saturation, vignette, grain, aberration})`, `reset()` |
| `lights` | `set({{pos, color, intensity, range}, ...})` (at most 32), `clear()` |
| `sky` | `set(name)`, `get()` |
| `light` | `set({sun, sun_lux, sky_lux, exposure, fog_start, fog_end, fog_density})`, `reset()` |
| `random` | `float()`, `range(lo, hi)`, `int(lo, hi)`, `chance(p)`, `seed(n)` |
| `game` | `tick()`, `time()`, `dt()`, `players()`, `title()`, `finish()`, `log(text)` |

- **Entities**: `world.spawn` takes a shape (`"cube"`, `"sphere"`,
  `"capsule"`, or a glTF path inside the game folder such as
  `"assets/crate.glb"`) and options: `pos`, `rot` (pitch, yaw, roll),
  `size` (a number or a vector; 1 fills a 1 m box, and the capsule is 2 m
  tall), `color`, `emissive` (0 off, 1 a clear glow in daylight),
  `roughness`, `metal`, `solid` (default true), `visible` and `tag`.
  `world.set` takes the same options plus `shape`. `world.place(id, pos,
  yaw?)` only moves (and turns) an entity: the cheap call for things that
  move every tick. `world.warm(shape, {color, emissive, roughness, metal})`
  prepares a look in `init` without spawning anything (see
  [Costs](#costs)).
- **Effects**: `fx.burst(pos, opts)` throws `count` pieces (default 12
  cubes, 0.12 m) out from `pos`, each at 25 % to 100 % of `speed` (m/s) and
  living 50 % to 100 % of `life` (s), shrinking as they go; `flat` keeps
  them in the ground plane, `stretch` (seconds of velocity) draws them as
  streaks, `gravity` pulls them down. `fx.ring(pos, opts)` grows a ring of
  `segments` thin cubes in the ground plane out to `radius` over `life`.
  The engine moves them every tick and removes them when spent; they are
  entities tagged `fx`, never solid, and are skipped when the world is
  full. Their look needs warming like any other.
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
  `hud.project(pos, player?)` is where a world point shows (in that
  player's view, by default the first), for labels over things; nil when
  it is behind the camera.
  Script and patch errors show in red at the bottom of the screen until
  they are fixed.
- **Sky and light**: `sky.set("sunset:haze")` bakes a procedural sky
  (about 0.2 s, on the next frame) and sets its sun, skylight, fog and
  exposure. `light.set` overrides parts of that.
- **Post and lights**: `post.set` is the look after lighting: `bloom`
  (0 to 2; the glow is screen-blended, Godot style), `bloom_radius` (blur
  rounds 1 to 6), `bloom_threshold` (exposed brightness where it starts),
  `contrast`, `saturation`, `vignette` (0 to 1), `grain` (film grain, 0 to
  1) and `aberration` (pixels of red and blue apart at the corners).
  `lights.set` replaces the coloured point lights (no shadows): `intensity`
  is roughly lux at 1 m, `range` in metres. Under the `night` sky a few
  hundred lights a dark floor well. Both are looks only, allowed in `draw`
  too, and reset on restart. Neon Siege and Neon Trails show a neon look:
  deep colours at a low emissive (0.03) with soft bloom, a glossy dark deck
  and a lamp over each player.
- **Random**: SplitMix64 from the game's seed. `restart` reseeds it.

### Players

`players.list()` returns every player, people first in split-screen
order, then bots: `{id, name, color, bot, human, seat, couch}`. Ids start
at 1, stay the same for the whole session and are never reused, so a
player who leaves does not renumber the others. Each person has a camera
and a view (`camera.set(p.id, ...)`, `hud.view(p.id)`); bots have none
(setting a bot's camera does nothing visible). With only bots, the first
bot's camera fills the screen, which makes a demo. The scripts drive bots
themselves; the starter shows a simple one that runs to the nearest orb.

Without a party host the players come from `game.toml` (`players`,
`bots`, `--players`, `--bots`) and are called "Player 1" and "Bot 1".
`min_players` adds bots in both cases.

### The start menu

Start on a controller (Enter or Escape on the keyboard) opens the menu
every GameNight game has: the game's title, Resume and New round (and Quit
without a party host; under GameNight the host's own overlay leaves the
game). No button hints. While it is open the game is frozen: `tick` does not
run. New round runs `restart`. Up and down (or the stick) choose, A
confirms, B or Start closes. Scripts and agents use the console:
`menu`, `menu new round`, `menu resume`, `menu close`.

### GameNight

`warp run` and `warp DIR` connect to a party host when one launched them
(`GAMENIGHT=1`) or `GAMENIGHT_ADDR` points at one, using the
[`gamenight` crate](../crates/gamenight/src/lib.rs) as Growing Guns does:

- The window stays hidden until the host starts the session, then goes
  fullscreen.
- `prepare` starts the game over (`restart`) with the host's seats as
  players: a couch seat is a person, an AI seat a bot, with the lobby's
  name and colour. The game reports ready and does not tick until `start`.
- The seats' controllers are `input[id]`. Back belongs to the host. A host
  pause freezes the game; `dispose` clears the world and waits again.
- Names and colours follow the lobby live (`players_changed`). A seat added
  or removed mid-game joins or leaves without a restart (`player_joined`,
  `player_left`); at most four people play.
- `game.finish()` tells the host the match is over, `game.log(text)` writes
  a line in the room log in the app, and `players.rumble(id, strength,
  seconds)` buzzes a person's controller. Without a host, finish and log
  only print.

`state` shows `players` and `phase` (`standalone`, `waiting`, `ready`,
`running` or `paused`).

### State

Every module has a global `state` table that survives reloads. **main.luau's
`state` is the game's state**: `state()` reports it as JSON under
`/state`. Scripts check it with `expect` (`/state/score`), and MCP
clients read it with `state` and `query`. Numbers, strings, booleans,
vectors (`[x, y, z]`) and tables become JSON. Functions are left out.

The engine adds its own fields next to it: `tick`, `time`, `players`,
`phase`, `menu`, `sky`, `entities`, `tags` (entities per tag), `cameras`, `sounds`
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
for things that come and go, as the starter's orbs do, and warm every
other look in `init` with `world.warm` (Neon Siege warms each enemy's
normal, spawning and hit looks, every player's colour and the effects).

Luau runs every tick under a step budget (`steps` in game.toml). Neon
Siege, with 96 enemies, 600 shots, their hits and the bursts, takes about
half a millisecond a tick.

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

- unit tests: `game.toml`, sky names, Luau-to-JSON, party colours and pads, the world (spawning,
  mesh reuse, rays, overlaps, the mover) and colour parsing;
- `tests/game.rs`, without a GPU: the starter equals games/hello-warp, `warp
  new` writes a game that plays and jumps, the same seed replays exactly,
  the API (rays, overlaps, typos, seeded numbers, the console), hot reload
  (state kept, `reload()` instead of `init`, a broken edit keeping the last
  version, runtime errors, restart), patch reload and playback, bots in
  split screen (replaying exactly), `min_players`, the start menu, effects
  (bursts and rings spent after their life, `world.place`, `aim_x`), Neon
  Siege with bots clearing waves and replaying exactly, and a
  fake GameNight host on loopback (prepare, ready, start, a seat's
  controller, a live rename, a seat joining mid-game, the room log and
  `finished`);
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

- Skinned and animated glTF, local lights, physics bodies
  (the `physics` crate) and terrain are not exposed to Luau yet.
- No `require`: modules cannot share code.
- No online play. The determinism rules are meant to make lockstep or
  host-authoritative play possible later.
- `game.toml` and assets do not hot-reload.
- No interpolation between ticks: entities move at the tick rate.
- GameNight party settings (`declare_settings`), phone buttons
  (`declare_actions`) and per-player settings are not exposed to Luau yet.
