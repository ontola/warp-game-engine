# The warp executable: games in Luau

`warp` runs a game made only of Luau scripts, sound patches and assets.
There is no Rust to write and nothing to recompile: edit a file while the
game runs and it reloads. This is the second way to make a game on Warp,
next to a Rust crate (see [making a game](making-a-game.md)). Sideways and
Growing Guns stay crates. A `warp` game suits quick ideas and live
iteration with an AI over MCP.

The crate is `engine/crates/warp`. The example is
[games/hello-warp](../../games/hello-warp/README.md), the starter that
`warp new` writes. [games/neon-trails](../../games/neon-trails/README.md)
and [games/neon-siege](../../games/neon-siege/README.md) are LÃ–VE party
games ported to Luau, with one shared view; Neon Siege is a co-op
twin-stick shooter with hundreds of shots, bursts and rings at once.

```sh
cargo build --release -p warp          # target/release/warp
warp new my-game                       # a starter game
warp run my-game                       # play fullscreen (--windowed to opt out)
warp run my-game --live                # ... and let Claude change it over MCP
```

## Commands

| Command | What it does |
| --- | --- |
| `warp run DIR [--players N] [--bots N] [--live [PORT]] [--sky SKY] [--seed N] [--size WxH] [--fullscreen] [--frames N]` | Play fullscreen, with sound (`--windowed` opts out). `--live` serves MCP ([live control](live-control.md); all its `--live-*` flags work). `--frames N` quits after N frames (a smoke test). Launched by a Splitscreen Club party host, the host's seats decide the players (see [Splitscreen Club](#splitscreen-club)). |
| `warp DIR` | The same as `warp run DIR`. This is how a Splitscreen Club host starts an installed game: the runtime gets the game folder. |
| `warp script DIR FILE.json --out OUT [--players N] [--sky SKY] [--seed N]` | A headless playtest in the [`app::script` format](building-and-running.md): ticks with controls, `cmd` lines, `expect` and `until` checks, `capture` (PNG), `state` (JSON) and `audio` (WAV). Exits with status 1 when a check fails. |
| `warp serve DIR [--live [PORT]] [--seconds T] [--size WxH]` | Headless in real time with live control on (port 7878 by default). Renders a frame only for a screenshot. Sound plays into a mixer that is never heard. |
| `warp check DIR [--ticks N]` | Load, run `init` and N idle ticks (default 120) without a GPU, then print the state. Exits with status 1 on any script or patch error. The fastest check of an edit. |
| `warp samples DIR [--bundle] [--credits]` | Fetch the recordings `samples.lock` lists into the cache, checking each sha256 (`--bundle`: into `DIR/samples/` for a package; `--credits`: who made them). See [audio.md](audio.md#recorded-samples-in-a-warp-game-sampleslock). |
| `warp audio render FILE.luau [...]`, `check FILES`, `units` | The [`warp-audio`](audio.md#warp-audio) tool: render a patch and print its levels and a text spectrogram, check patches, list units. |
| `warp new DIR [--title TITLE] [--template NAME]` | Write the starter game into a new folder; `warp new GAME.luau` writes it as [one file](#a-game-in-one-file). `--template platformer` (or `fps`) starts from a whole game built on the [game kit](kit.md) instead, with the kit's guide as `KIT.md`. |
| `warp templates` | List the templates. |

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
| `storage` | false | Enable [versioned local saves, settings and profiles](storage.md). Requires an explicit game ID and a schema such as `save-schema.json`. |
| `id` | the folder name | The Splitscreen Club game id (lowercase letters, digits, dashes). A host's `GAMENIGHT_GAME_ID` wins. |
| `players` | 1 | People without a party host, 1 to 4, in split screen. `--players` overrides it. |
| `split` | true | One split-screen view per person; false gives one shared view (every `camera.set` moves that camera), as in an arena game. |
| `bots` | 0 | Bots without a party host, 0 to 7. `--bots` overrides it. |
| `min_players` | 1 | Bots fill up to this many players, with or without a party host (a duel needs two). 1 to 8. |
| `seed` | 1 | Seed of the `random` generator. `--seed` overrides it. |
| `fresh_seed` | false | A new seed for every round played: `warp run` picks one at launch and New round in the start menu another. `--seed`, scripts and tests keep `seed`. |
| `sky` | `gradient` | The sky at start. `--sky` overrides it; scripts call `sky.set`. |
| `room_lighting` | false | Enable shadowed panels and up to four streamed indoor lighting volumes. [Luau room lighting](interior-light.md#streamed-ground-floors-in-luau) |
| `max_entities` | 1024 | Entities alive at once (render slots reserved up front). |
| `size` | `[1280, 720]` | Window size in pixels. |
| `hdr` | unset | `true` asks for HDR output on an HDR screen (highlights brighter than SDR white), `false` refuses it; unset follows `WARP_HDR`. F10 overrides it. See [hdr-and-exposure](hdr-and-exposure.md). |
| `split_layout` | `"stacked"` | How two views share the screen: `"stacked"` (top and bottom) or `"side"` (left and right). |
| `steps` | 200000 | Luau step budget per hook call (see [scripting](scripting.md#limits)). |
| `memory_mb` | 64 | Luau memory cap. |
| `[humans.NAME]` | none | A pool of Warp Humans: `count` (1 to 256 alive at once), `variety` (different people fitted at load, 1 to 16, default 4), `style` (`"person"` or `"zombie"`), `skin` and `shirt` (`"#rrggbb"`, default seeded), `height` (`[1.6, 1.9]` m), `age` (`[low, high]` in years, 18 to 100; the pool's people are spread over it; default seeded, mostly young adults), `seed`. At most 256 in all. Each one costs about 2 MB of GPU skinning memory, so on a device that binds less than all pools need (128 MB on some), the engine keeps each pool’s `reserved` minimum (default 1) and shares the remaining capacity proportionally. A line in the log reports the resulting counts. |

Regular scripts outside `sounds/`, `assets/` and `scripts/` run their hooks
in path order. Type definitions (`*.d.luau`) are skipped.

### Local libraries

In a game folder with `main.luau`, use a `.module.luau` file to share
functions or data. For example, put this in `lib/score.module.luau`:

```lua
return { winner_points = function(rounds) return rounds * 3 end }
```

At the top of any hook script or library, write
`local score = require("lib/score")`. Paths start at the game root and omit
the suffix. The loader looks up exact names among the discovered libraries.
Use the returned table in hooks, for example `score.winner_points(2)`.
A library's exports do not register hooks. The [Neon Trails grid](../../games/neon-trails/grid.module.luau)
is a working example.

Declare imports while the file loads. Each hook script gets its own cache:
two imports of a library in that script's dependency tree return the same
table. Keep persistent match data in the hook script's `state`. Editing a
library reloads every dependent hook script, including indirect dependents,
and runs its normal reload hook. A missing library, a cycle or a bad edit
reports a load error and keeps the last working exports. Fixing the library
retries the load. One-file games remain self-contained.

For content collections, where each new file is a new entry (cards,
encounters, levels), `libraries("content/cards")` returns the names of the
libraries in that folder, sorted, so a game can load them all without a
list to keep up to date:

```lua
local CARDS = {}
for _, name in libraries("content/cards") do
	table.insert(CARDS, require(name))
end
```

[Dead Miles](../../games/dead-miles/content/encounters) loads its
encounters this way.

## The game kit

Before writing movement, cameras, a HUD or match flow yourself, use the
[game kit](kit.md): Luau libraries built into `warp` that any game loads
with `require("kit/controller")` (also a one-file game). Controllers for
platformers, top-down and twin-stick games, third person and karts; camera
rigs; HUD pieces; countdown, scores and results; bot inputs; timers and
tweens.

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
| `menu(y)` | Every displayed frame while the start menu is open, instead of `draw()`: `hud` calls draw over the menu's backdrop, from `y` (pixels, under its last item) down. With this hook the menu sits higher. |
| `reload()` | After a saved edit has reloaded the module. |
| `player_joined(p)` | A Splitscreen Club seat was added mid-game (`p` is a player table). |
| `player_left(p)` | A player left mid-game. |
| `players_changed()` | Names, colours or seats changed, after any join or leave too. |
| `couch_changed(order)` | The people's left-to-right order on the couch changed: someone moved in the lobby mid-game, joined or left. `order` is `players.by_couch()`. Runs after `players_changed`. See [couch order](#couch-order). |

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
| `world` | `spawn(shape, opts, parent?) -> id`, `set_parent(id, parent?)`, `set(id, opts)`, `place(id, pos, yaw?)`, `warm(shape, look)`, `get(id) -> entity or nil`, `exists(id)`, `remove(id)`, `clear()`, `find(tag) -> ids`, `count()`, `raycast(origin, dir, max?, query?) -> hit or nil`, `overlap(center, radius, query?) -> ids`, `move(id, vel, dt, query?) -> vel, grounded` |
| `fx` | `warm_explosions()`, `explosion(pos, radius, lightRange?)`, `gib_mist(pos, intensity)`, `flame(pos, width, height, life)`, `exhaust_smoke(pos, radius)`, `burst(pos, {color, emissive, shape, count, speed, life, size, stretch, gravity, flat})`, `ring(pos, {color, emissive, radius, life, width, segments})`, `count() -> particles, rings` |
| `players` | `list() -> players`, `get(id) -> player or nil`, `count()`, `rumble(id, strength, seconds?)` |
| `camera` | `set(player, {pos, target, fov})`, `get(player)`, `views({[player] = view})` (who shares which part of a split screen; views from 1, so a game can split when players walk apart) |
| `sound` | `play(name, {at, db, pitch, <param> = value}) -> handle`, `set_param(handle, param, value)`, `stop(handle)`, `list()` |
| `hud` | `text(text, x, y, {size, color, align, width})`, `box(x, y, w, h, color)`, `size() -> w, h`, `measure(text, size?) -> w, h` (one line of `text` in the HUD font, in pixels), `view(player) -> x, y, w, h`, `project(pos, player?) -> x, y or nil` |
| `post` | `set({bloom, bloom_radius, bloom_threshold, bloom_wide, contrast, saturation, vignette, grain, aberration})`, `reset()` |
| `grass` | `set({patches, cutouts, seed, growth, distance})` places native meadow blades on horizontal plots. `set(nil)` clears the field. |
| `plants` | `grow({age, seed, pos, yaw, scale, species, look, full})` → id, `remove(id)`, `clear()`, `defaults()`, `status()` |
| `humans` | `spawn(pool, {pos, yaw})` → id or nil when the pool is full, `set(id, {pos, yaw, pose, t, tired, coil, visible})`, `hit(id, dir)`, `ragdoll(id, push)`, `obstacles(boxes)`, `recover(id)`, `remove(id)`, `head(id)` and `joint(id, name)` → vector, `room(pool)` → free, total |
| `lights` | `set({{pos, color, intensity, range}, ...})` (at most 32), `spots({{pos, dir, color, intensity, range, angle, inner}, ...})` (at most 8, with shadows), `clear()` |
| `sky` | `set(name)`, `get()`, `warm(name)` (bake a sky you will switch to later in the background, so the switch does not stall a frame) |
| `light` | `set({sun, sun_lux, sky_lux, exposure, exposure_min, exposure_max, exposure_bias, adapt_speed, fog_start, fog_end, fog_density})`, `reset()` |
| `random` | `float()`, `range(lo, hi)`, `int(lo, hi)`, `chance(p)`, `seed(n)` |
| `game` | `tick()`, `time()`, `dt()`, `players()`, `title()`, `finish()`, `log(text)` |

- **Parenting**: pass a parent ID as the third argument of `world.spawn`. Move a subtree with `world.place`, change its parent with `world.set_parent` and read `world.get(id).world_pos`. See [parenting](parenting.md) for coordinates, collisions and lifetime.
- **Entities**: `world.spawn` takes a shape (`"cube"`, `"pane"`, `"sphere"`,
  `"capsule"`, an outline such as `"outline6"` or `"outline4:0.05"`, or a
  glTF path inside the game folder such as `"assets/crate.glb"`) and
  options: `pos`, `rot` (pitch, yaw, roll),
  `size` (a number or a vector; 1 fills a 1 m box, and the capsule is 2 m
  tall), `color`, `emissive` (0 off, 1 a clear glow in daylight),
  `roughness`, `metal`, `solid` (default true), `visible`, `shadow_only`
  (casts shadows, but no camera sees it: a ceiling over a roofless room
  that keeps the moon out while the camera looks in), `hide_views` (a mask:
  bit i hides it from split-screen view i only, shadows stay: a roof that
  opens for the view whose player went inside) and `tag`. An
  outline is a hollow regular polygon lying flat (3 to 64 sides, corners
  on the 1 m box's circle, the first towards +X) whose edge is the given
  share of the box wide (default 0.06): neon line art seen from above.
- **LOD proxy coverage.** On an opaque primitive, `coverage` from 0 to 1 controls
  the fraction of pixels that keep the proxy and its depth. The GPU uses a stable
  ordered pattern. The opaque detailed geometry behind it supplies the remaining
  pixels. Proxies cast no shadows and do not enter room light bakes. Collision
  follows `solid` separately. Use `world.warm("cube", look)` with `coverage=1`
  to prepare one proxy material for every coverage value. Update it through
  `world.set`; `coverage=false` restores the ordinary material. For independent
  split-screen transitions, assign proxy copies to cameras with `hide_views`.
  `players.list()` exposes each displayed player's zero-based `view` index.
- **Window glass** uses a two-sided `"pane"` in local XY. For a 3 m wide,
  2 m high window, spawn it with `size = vector.create(3, 2, 1)`. Its
  collider is 4 mm thick at unit depth. Set `glass = 0.045` for lightly
  scattering window glass, or use any scattering opacity from 0 to 1.
  `color` sets the tint and `roughness` controls reflection sharpness.
  `world.warm("pane", look)` prepares the material before streaming windows.
- **Generated meshes.** Call `local id, err = world.mesh(parts, opts)` from simulation code to draw a procedural mesh as one clustered entity. Each material part supplies `positions` and `normals` (equal-length lists of local vectors), and `indices` (flat, 1-based triangle triples). Optional `uvs` use `vector.create(u,v,0)` per vertex. Set `color`, `surface`, `roughness` and `metal` on each part. The entity options are `pos`, `rot`, `size`, `visible`, `tag` and `solid`. A mesh defaults to visual only. Setting `solid=true` gives it box collision; `world.heightfield` supplies ground contacts for a triangulated terrain. One mesh accepts up to 32 parts, 65,536 vertices and 196,608 indices. The aggregate live vertex and index limits are four times those per-mesh limits. Invalid geometry returns `nil, err` and preserves the scene. Move the entity with `world.place` or `world.set`. To replace its geometry, create the new mesh, check its returned ID, then remove the old entity. Removal releases its storage for reuse.
- **Wires**: `world.spawn("wire", { points = {...}, edges = {...}, width = 0.05 })`
  is a 3D wire frame: glowing edges between points, one entity with one
  transform. `points` are vectors in the entity's own space (scaled by
  `size`, turned by `rot` in all three axes), `edges` pairs of 1-based
  point numbers (`{ {1, 2}, {2, 3} }` or flat `{ 1, 2, 2, 3 }`), `width`
  the edge's thickness in metres at size 1. Each edge takes one entity
  slot (count them in `max_entities`). Tumble it with `rot`; animate it
  (legs, tails, a breathing hull) by setting new `points` with `world.set`
  each tick: the engine moves every edge, so one call per wire is enough.
  It is a cube look, so `world.warm("wire", look)` and `world.warm("cube",
  look)` warm the same thing.
  `world.set` takes the same options plus `shape`. `world.place(id, pos,
  yaw?)` only moves (and turns) an entity: the cheap call for things that
  move every tick. `world.warm(shape, {color, emissive, roughness, metal})`
  prepares a look in `init` without spawning anything (see
  [Costs](#costs)).
- **Effects**. `fx.explosion(pos, radius)` plays the Growing Guns fireball, smoke,
  embers, heat distortion and flash lighting in every view. Call it once in
  init or tick with a radius in metres (0 < radius ≤ 200). The
  effect pool admits bursts within Growing Guns' low-cost budget. Full-lighting
  devices that support the 48 × 48 × 1,536 smoke texture use the shared baked
  smoke animation. Simple-lighting profiles, smaller devices and blasts admitted
  before a renderer exists use Growing Guns' procedural smoke billows.
  To bound the flash near an impact, pass a third argument in metres (1–500).
  For example, `fx.explosion(pos, 5, 12)` lights surfaces within 12 metres
  using inverse-square falloff while retaining the five-metre blast recipe.
  `fx.gib_mist(pos, intensity)` emits the Growing Guns impact mist, with intensity from 0.35 to 2.8.
  A moving fire can emit short `fx.flame(pos, width, height, life)` tongues and rising
  `fx.exhaust_smoke(pos, radius)` puffs. Their dimensions are in metres and lifetime
  is in seconds. They share the scene's effects pass and consume no entity slots.
  Call `fx.warm_explosions()` in `init` to upload the selected smoke and draw
  the first blast offscreen during loading. Games supply sounds and gameplay
  responses separately.
  `fx.burst(pos, opts)` throws `count` pieces (default 12
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
  `hud.button(p, "Y", x, y, size)` draws a button as player `p` presses
  it: the coloured A/B/X/Y disc on a controller, the key's cap ("E", "'")
  on a keyboard or a keyboard half, so prompts and control lists are right
  for everyone; `hud.key(p, "Y")` is that key's name, or nil on a pad.
  Script and patch errors show in red at the bottom of the screen until
  they are fixed.
- **Sky and light**: `sky.set("sunset:haze")` bakes a procedural sky
  (about 0.2 s, on the next frame) and sets its sun, skylight, fog and
  exposure. `light.set` overrides parts of that. `exposure` is a number
  (EV100: sunny daylight about 15, a lit room about 7) or `"auto"`: the
  camera meters each player's view and adapts like an eye, within
  `exposure_min` and `exposure_max`, shifted by `exposure_bias` stops, at
  `adapt_speed` per second. See [Exposure and HDR output](hdr-and-exposure.md),
  which also covers HDR screens (`WARP_HDR=1`, or F10).
- **Runtime stories.** A game may export `apply_story(definition)` from its main module. Accept a validated definition by returning its title, or return `nil, message` to keep the current story. In the native console `story SOURCE` evaluates a Luau program returning plain data. The browser story editor evaluates the same language in a disposable worker, then sends the data to the game. Put a list of `{"title":"…","file":"story_name.module.luau"}` entries in the game's `stories.json` to expose its editable templates. [Example city and authoring guide](../../games/warp-theft-auto/README.md)
- **Continuous daylight.** Call `sky.set_time(hour, weather)` from the world clock. Fractional hours wrap at midnight. All cameras share the sun direction, sky radiance and diffuse skylight. The renderer interpolates between simulation ticks and reuses the same atmosphere textures as the clock advances. `kit/daycycle` drives this API and exposes `lampLevel` to fade street lights through twilight.
- **Volumetric clouds.** Call `sky.clouds(true)` to load the native cumulus layer or supply a table such as `sky.clouds({coverage = 0.5, base = 1400, thickness = 1000, wind_x = 5, wind_z = 1.5, aerial = false})`. Altitudes and distances are in metres, wind in metres per second. `coverage`, `density` and `shadow` control cover, optical density and sun attenuation. `steps`, `light_steps` and `far` control rendering cost and range. Set `aerial = false` to keep the current scene fog. The field follows simulation time across cameras and sky presets. `sky.clouds(false)` clears the volume. [Atmosphere and clouds](atmosphere.md)
- **Post and lights**: `post.set` is the look after lighting: `bloom`
  (0 to 2), `bloom_radius` (blur rounds 1 to 6), `bloom_threshold`
  (exposed brightness where it starts), `bloom_wide` (true, the default: a
  wide screen-blended glow, Godot style; false: a tight halo added around
  what glows, for neon lines),
  `contrast`, `saturation`, `vignette` (0 to 1), `grain` (film grain, 0 to
  1) and `aberration` (pixels of red and blue apart at the corners).
  `lights.set` replaces the coloured point lights (no shadows): `intensity`
  is roughly lux at 1 m, `range` in metres. Under the `night` sky a few
  hundred lights a dark floor well. `lights.spots` replaces up to eight
  spotlights that cast shadows (torches, headlights, a street lamp):
  `dir` is the beam, `angle` its outer half-angle in degrees (2 to 74),
  `inner` where the edge starts to fade, `intensity` in candela with
  inverse-square falloff (a hand torch is about 1500), `range` in metres.
  With `parent = entityId`, `pos` is an offset in local metres and `dir` a
  local direction. Both follow the entity's interpolated render pose. Mesh
  scale has no effect on these offsets. Removing the entity also stops its
  light. Omit `parent` for world-space lights. Games targeting older hosts
  can check `lights.supports_parent` before supplying the field.
  Each spot renders a 1024-square shadow map every frame, so use the ones
  that matter (Dead Miles: one per survivor's torch, then the street lights
  nearest the view). All three are looks only, allowed in `draw`
  too, and reset on restart. Neon Trails shows a neon look: deep colours
  at a low emissive (0.03) with soft wide bloom, a glossy dark deck and a
  lamp over each player. Neon Siege is Geometry Wars style: outlines at
  emissive 0.14 under a tight bloom, a camera straight down, no sky light.
- **Grass.** Call `grass.set` during world generation. Replacing the field rebuilds the scene. Supply each horizontal rectangle as `{pos = vector.create(x, y, z), width = 18, depth = 12}`. `pos` is the centre at ground level. An optional `yaw` in radians rotates the rectangle around that centre. `cutouts` use the same shape for paths, paving or grates. All patches share the seed, growth scale (default 0.3) and blade draw distance (default 24 m). The GPU chooses detail for each camera. Keep the lawn’s ground material underneath for distant views.
- **Plants**. `age` controls the number of growing seasons. Choose a small
  set of ages, seeds, growth rules and looks for a grove. Placements with
  the same recipe share a cook. Set each placement’s `pos`, `yaw` and
  positive uniform `scale` to vary its position, facing and size. Scale
  defaults to 1 and applies to both the mesh preview and the full forest.
- **Humans**: Warp Humans, the engine's skinned people, from pools that
  game.toml reserves (`[humans.zombie] count = 40, style = "zombie"`), so
  spawning never rebuilds the renderer. Set each one's feet `pos` and
  `yaw` (0 faces +z, a quarter turn +x) every tick: the legs walk, jog,
  sprint (above about 5 m/s) or stand from how far it moved. Standing, a
  person is upright and relaxed (arms by the thighs, breathing, the weight
  drifting, the head looking about, each in their own time); the dead keep
  the capture's low, wide stance. Every person is posed on their own
  skeleton: the legs are the rig's, the spine, head, shoulders and arms sit
  where that body has them, so `joint` answers differ a little from person
  to person. `pose` lays an action over that: `walk`
  (the default), `idle`, `crouch` (low: waiting on one knee, or walking
  crouched when it moves), `wave` (the right hand high, waving), `reach` and `raise` (arms ahead, arms over the
  head), `strike` (from raised to low over `t`), `aim`, `shove`,
  `attack`, `punch`, `jab`, `hit`, `hit_head`, `cast`, `roll` (captures
  played by `t`, 0 to 1), `dead` (the fall over `t`) and `down` (lying).
  `tired` (0 to 1) bends anyone over, out of breath, over any pose: the
  chest heaves and the arms hang. `coil` (-1 to 1) winds up a heavy blow:
  at 1 the chest turns the right shoulder back and leans away while the
  head keeps facing ahead; below 0 it swings through past the front.
  `hit(id, dir)` jolts the upper body
  along a blow (`dir` in the world, its length the strength, about 1 for
  a heavy hit): the chest and head swing that way and spring back.
  `ragdoll(id, push)` lets it go limp from the pose it stands in, thrown
  by `push` (m/s, in the world): eleven jointed capsules with loose
  anatomical limits fall onto a horizontal floor at the character's feet.
  Pass an explicit third vector to release the rig with exact world linear and
  angular velocity, e.g. `humans.ragdoll(id, walkingVelocity, vector.create(0, 0, 0))` for
  a walker meeting a moving collider. Angular velocity is in radians per second.
  Up to 24 rigs move at once on fixed physics ticks. A rig freezes when it
  settles on fixed ground or a new fall needs the oldest active slot. After four
  seconds the settling test tolerates small joint motion. A parked car supports
  recovery while its passenger stays simulated until actually getting up.
  `humans.obstacles(boxes)` supplies shared moving or parked collision boxes.
  Give each box a stable numeric `id`, world center `pos`, full `size` and
  optional `yaw` in radians. Supply `prevPos` and `prevYaw` for a first-tick
  sweep. With `vehicle=carId` an obstacle uses the fleet car's actual compound
  chassis at its consecutive road-physics poses, including pitch and roll.
  To give a fleet model a more detailed contact skin, register
  `vehicles.contact_profile(name, bottom, sections)` after `vehicles.reset`
  and include `profile=name` on its obstacle. Describe 2 to 20 ordered
  cross-sections with local `z`, full `width`, `top`, and optional `roofWidth`.
  All lengths use metres relative to the chassis origin. Cars share one cooked
  surface for each name. A moving chassis and its nearby ragdolls advance together
  in up to four substeps per fixed tick, based on the distance travelled by the
  contact skin. This lets the solver follow the bumper, bonnet and windscreen.
  The box remains its proximity and standing-clearance bound. Pass up to 64 per tick. Only boxes near an active rig allocate
  physics bodies. An empty list clears them. `humans.recover(id)` waits for
  a settled body and clear space, returning nil while either is pending.
  On success it returns the landed feet, starts `rise` at t=0 and releases
  the ragdoll. Advance `t` from 0 to 1, then resume walking. Setting a pose
  other than `dead` or `down` also ends a fall at the script's current position. The `zombie` style stoops, lolls its head and holds its arms out.
  `head(id)` is where the head is now (for glowing eyes or a hit test);
  `joint(id, name)` any of `head`, `neck`, `chest`, `hips`, `shoulder_l`,
  `elbow_l`, `hand_l`, `knee_l`, `foot_l` (and `_r`), say to put a weapon
  in a hand.
  Set `visible = false` to hide the fitted actor while a player rides in a
  vehicle. The actor keeps its identity, slot and joint queries. Restore
  `visible = true` at the exit position to show that same person again.
  Looks only: they never collide, and the game keeps its own positions.
  Each human is a full skinned mesh skinned every frame, so keep pools to
  the crowd that is on screen. The rig is in `engine/assets/humans/rig`;
  the captures are the `body` crate's. `games/human-lab` shows a row of
  them up close from fixed portrait cameras, for judging looks.
- **Random**: SplitMix64 from the game's seed. `restart` reseeds it.

### Players

`players.list()` returns every player, people first (in couch order when
the game starts), then bots: `{id, name, color, bot, human, seat, couch}`. Ids start
at 1, stay the same for the whole session and are never reused, so a
player who leaves does not renumber the others. Each person has a camera
and a view (`camera.set(p.id, ...)`, `hud.view(p.id)`); bots have none
(setting a bot's camera does nothing visible). With only bots, the first
bot's camera fills the screen, which makes a demo. The scripts drive bots
themselves; the starter shows a simple one that runs to the nearest orb.

Without a party host the players come from `game.toml` (`players`,
`bots`, `--players`, `--bots`) and are called "Player 1" and "Bot 1".
`min_players` adds bots in both cases.

#### Couch order

Players choose where they sit on the couch in the Game Room lobby (Start
menu, Couch position), also while a game runs. `couch` is that place, 0
leftmost; it is nil without a lobby (then roster order counts).
`players.by_couch()` returns the players left to right: people by
`couch`, then bots. The engine does the rest by itself:

- At the start, `players.list()` already lists the people in couch order.
- Split-screen views follow the couch: who sits left looks through the
  left view, also after a move mid-game (a game that does nothing still
  gets this right).
- On a shared keyboard the browser host gives who sits left the left half
  (WASD) and who sits right the arrows, and swaps them after a move.
- When the order changes mid-game, `couch_changed(order)` runs. A game
  whose layout depends on where people sit (lanes, halves of a board,
  sides of an arena, HUD columns) moves them there; Stack Together swaps
  who plays which half of the well and keeps the stacks.

`players.list()` itself keeps its order during a game, so indices a game
stored stay valid; use `by_couch()` wherever left and right matter.

Rust games on the `app` shell use [`app::couch`](../crates/app/src/couch.rs):
each local player's couch place comes from the party's seats
(`splitscreen_club::Seat::couch`, updated by `players_updated`) or, in the
browser and phone hosts, from `app::seat_couch(seat)`, which the host sets
every tick. `app::couch_views(&couch)` gives each player's view and
`app::in_view_order` puts cameras and per-view settings in that order.
Space Racer and Growing Guns do this (Growing Guns also re-masks own
bodies and guns per view after a move); Sideways sorts its views by couch
itself.

### The start menu

Start on a controller (Enter or Escape on the keyboard) opens the menu
every Splitscreen Club game has: the game's title, Resume and New round (and Quit
without a party host; under Splitscreen Club the host's own overlay leaves the
game). No button hints. While it is open the game is frozen: `tick` does not
run. New round runs `restart`. Up and down (or the stick) choose, A
confirms, B or Start closes. Scripts and agents use the console:
`menu`, `menu new round`, `menu resume`, `menu close`.

### Splitscreen Club

`warp run` and `warp DIR` connect to a party host when one launched them
(`GAMENIGHT=1`) or `GAMENIGHT_ADDR` points at one, using the
[`splitscreen-club` crate](../crates/splitscreen-club/src/lib.rs) as Growing Guns does:

- The window stays hidden until the host starts the session, then goes
  fullscreen.
- `prepare` starts the game over (`restart`) with the host's seats as
  players: a couch seat is a person, an AI seat a bot, with the lobby's
  name and colour. The game reports ready and does not tick until `start`.
- The seats' controllers are `input[id]`. Back belongs to the host. A host
  pause freezes the game; `dispose` clears the world and waits again.
- Names, colours and couch places follow the lobby live
  (`players_changed`, and `couch_changed` when the order changed). A seat added
  or removed mid-game joins or leaves without a restart (`player_joined`,
  `player_left`); at most four people play.
- `game.finish()` tells the host the match is over, `game.log(text)` writes
  a line in the room log in the app, and `players.rumble(id, strength,
  seconds)` buzzes a person's controller. Without a host, finish and log
  only print.

`state` shows `players` and `phase` (`standalone`, `waiting`, `ready`,
`running` or `paused`).

The session lifecycle is shared by every Rust game that a party host
launches (Growing Guns, Spellstorm, Spaceracer and `warp`):
`splitscreen_club::party::Party` holds the client, the session id, the `Phase`
(`Idle`, `Warm`, `Running`, `Paused`) and the finished flag. It applies
`prepare`, `start`, `pause`, `resume` and `dispose` (only for the current
session), `ready` (progress, then ready), `finished` (once per session),
`room_log` and `loading`, and parses profile colours (`hex`, lenient;
`hex_strict`, exactly `#rrggbb`). `app::Screen::for_party` picks the window
state from the phase. A game wraps `Party` with what is its own: its
settings and specs, which events it emits for a poll, its seats, and the
mapping of a controller frame to `app::Controls` (`to_controls`, which
differs per game in buttons and look rate).

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

### Collision terrain from Luau

`world.heightfield({origin=vector.create(x,y,z), spacing=32, columns=20, heights=samples, tag="city", grip="grass", roads=routes})` creates a collision entity and returns its ID. Each row of samples runs along +X, successive rows along +Z. Heights are metres above the origin. The triangle diagonal connects the +X and +Z corners of each grid cell. Render the matching terrain separately.

Walkers, world rays and overlap queries use the grid. Call `vehicles.reset("city")` after installing the static terrain to include it in the fleet's collision snapshot. Each optional road has `points={{worldX,worldZ}, ...}`, a total `width` in metres, and a `grip` such as `"gravel"` or `"asphalt"`. The first matching road supplies tyre grip. Other terrain uses the base grip.

### Your own shader

Optional `lighting.wgsl` and `finish.wgsl` files control the lit colour and the per-view opaque image. See [Artist lighting and image effects](materials.md#artist-lighting-and-image-effects) for the signatures and a playable example.

A file `surface.wgsl` in the game folder is the game's own surface
shader. An entity (or `world.warm` look) with `surface = N` is drawn by it,
and `N` (0 to 255) tells its surfaces apart:

```lua
world.spawn("cube", { pos = vector.create(0, -11.5, 0), size = vector.create(80, 0.1, 50), emissive = 0.002, surface = 0 })
```

The file replaces the engine's
[game_surface.wgsl](../crates/render/src/shaders/game_surface.wgsl): copy
that file (it holds the `SurfaceIn` and `SurfaceOut` structs) and fill in
`fn game_surface(s: SurfaceIn) -> SurfaceOut`. It gets the world position,
the normal, the view direction, the entity's colour (`tint`, linear),
roughness and metal, `time` in seconds and `kind`; it returns the albedo,
normal, roughness, metal and an emission in nits. `scale` holds the
entity's `emissive` in nits (emissive Ã— 4000), so a Luau knob can set the
brightness. Prefix your own functions (`bd_noise`), because the file is
compiled together with the engine's shading code.

Branch on `SURFACE_LOW_DETAIL` in the shader to offer cheaper procedural materials.
Warp specializes this boolean when the renderer starts. Select it in the browser
with `?detail=low` (or `&detail=low` alongside other parameters), or set
`WARP_SURFACE_DETAIL=low` before launching the native game. Phones (the
browser's phone preset and the Android host, `game.phone()` in Luau) get it
by default; `detail=full` turns it off there. `WARP_PHONE=1` makes a desktop
run report `game.phone()` true, to try a game's phone tier.

An edit recompiles on the next frame, on screen and over MCP (`write_file`
takes `.wgsl`). A version that does not compile keeps the last good one
running and shows the compiler's message in `errors` until a good one
loads. `warp check` has no GPU and does not compile it; `warp script`
does. Neon Siege's tissue backdrop is an example
(games/neon-siege/surface.wgsl and backdrop.luau).

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
half a millisecond a tick. [Luau or Rust](luau-or-rust.md) has the cost of
each kind of call and when to move a loop into Rust.

## Hot reload

The game checks its files about four times a second (every 15 ticks) and
right away on the `reload` command or tool.

- **main.luau** (or any module): an edited file loads again. Its `state`
  table and every entity stay, so the game keeps playing with the new
  code. `init` does not run again; `reload()` runs instead, if the module
  exports it. To start over with the new code, use `restart`, which
  clears the world and every `state`, reseeds the random generator and runs
  `init`. Things `init` spawns once (an arena, a background) are not
  rebuilt by a reload, so a module that wants edits to them to show live
  removes them by tag in `reload()` and spawns them again (Neon Siege
  does this for its arena, stars and post settings).
- A load error (a syntax error, a module that returns something other than
  a table) keeps the previous version running and reports the file and
  line. A runtime error in a hook of a fresh edit puts back the version
  that ran before it (after 30 good calls a version counts as working); the
  message stays until the next good load. A runtime error in a version
  with nothing to fall back to turns that module's hooks off until the
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
(Luau against the live game; main.luau's globals, its exports and its
`state` are in scope, so `run_luau("debug.wave(12)")` calls a function
main.luau exports as `debug`), `list_files`,
`read_file` and `write_file` on the folder `game` (`.luau`, `.toml`, `.md`
`.json` and `.wgsl` files; a write reloads at once and the reply says whether the
file loaded), `logs`, `play_sound`, `set_param` and `render_sound`.

Warp's own tools:

| Tool | What it does |
| --- | --- |
| `reload` | Reload edited modules and patches now; returns what changed and any errors. |
| `restart` | Start over: clear the world and every `state`, reseed, run `init`. |
| `knobs` | The tuning knobs of main.luau: name, value, comment and line. |
| `set_knob` | `{name, value, persist}`: set one knob and reload. With `persist` (default true) the value is written into main.luau; with false only the running game changes. A value that does not load is put back and reported. |
| `keep` | Save temporary knob changes (`persist: false`) into main.luau. |
| `revert` | Undo temporary knob changes: run main.luau as saved. |
| `command` | One console line: `lua CODE`, `reload`, `restart`, `knobs`, `knob NAME VALUE`, `trial NAME VALUE` (temporary), `keep`, `revert`, `sky NAME`, `sounds`, `entities [TAG]`, `logs`, `help`. |

### Knobs

For clickable development commands, export [game-owned F10 editor menus](editor-tools.md#game-owned-editor-menus) from the entry module.

Tuning values a player wants to tweak while playing go in one block in
main.luau, one per line, between `-- @knobs` and `-- @end`:

```lua
-- @knobs
local KNOBS = {
  bloom = 0.55,      -- glow strength
  line_width = 3.2,  -- outline width in arena pixels
}
-- @end
```

`knobs` lists them and `set_knob` changes the value in the file itself,
so the change is saved and reviewable in git. A player trying values
from the Splitscreen Club app gets temporary changes instead (`persist: false`):
the game runs the edited source while the file stays as it was, until
`keep` saves them or `revert` (or an edit of the file on disk) drops them. The game reloads at once;
values that feed things `init` built only show when `reload()` rebuilds
them (see Hot reload).

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
  fake Splitscreen Club host on loopback (prepare, ready, start, a seat's
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

- Skinned and animated glTF (Warp Humans aside), physics bodies
  (the `physics` crate) and terrain are not exposed to Luau yet.
- Online play is host-authoritative and needs nothing from the game:
  `warp run DIR --host` and `--join TICKET` ([online couches](online.md)).
  Humans, vehicles and explosions are not replicated yet.
- `game.toml` and assets do not hot-reload.
- Splitscreen Club party settings (`declare_settings`), phone buttons
  (`declare_actions`) and per-player settings are not exposed to Luau yet.

### Native game visibility masks

Native Rust integrations can call `render::set_game_mask(source)` before renderer
construction. Provide WGSL `fn game_mask(world:vec3<f32>, instance:u32,
material:u32)->bool`; true discards that fragment in both opaque colour and depth
raster passes. The default keeps every fragment. The function may read scene
`aux` and `materials`; use an early range rejection for unrelated geometry. The
mask is fixed for a renderer lifetime (not a Luau API or hot-reloaded shader).
Reserve game-owned data in `Scene::aux` before engine reservations; update only
that range with `Renderer::update_aux_words`. Writes are bounds checked. This is
for opaque reconstruction/clipping, not forward transparency or ray-query masks.

## Sideways feedback

In Warp desktop games, press **F4** for feedback. Sideways also accepts **F12** to freeze the image and open the point overlay. Click or tap the problem to resume driving. Esc resumes with an unmarked capture. The overlay keeps the frame where and as it was (no letterbox; on an HDR screen in HDR, see [HDR output](hdr-and-exposure.md)) with the original HUD and the hint over it; on an HDR screen the capture adds a PQ `-hdr.png`. The writer reads the frozen pixels and source geometry, encodes the PNG and updates the clipboard on a background thread. Normal play keeps its existing resolution and render path. The JSON records request-to-overlay submission and input-ready timings in `feedback_timing`. A click saves `player_point: [x, y]` in original PNG pixels. The MCP feedback preview circles it. On a marked capture, `warp inspect F12.json` prints the point and its surface as JSON. Use `--preview` to view the ring or `--pixel X Y` to inspect another spot. Keep the PNG, JSON and same-basename
view attachments together. In a developer checkout with Python 3 and Pillow,
run the following commands. Set `WARP_PYTHON` to an interpreter path if needed.

```sh
warp inspect sideways-123.json
warp inspect sideways-123.json --pixel 320 180
warp compare sideways-123.json --baseline main --out build/drift-review
```

For an agent with the live MCP bridge, call `feedback` with
`{"diagnostics": false}` to get the latest PNG path and a preview. Open the
same-basename JSON on disk. Use pixel coordinates from the original PNG;
the MCP preview may be smaller. For headless inspection, `--pixel` prints JSON.
With only the JSON, replay and comparison work when `drive.version` is 1.
Inspection also needs the view attachments named in `visibility.views`.

### Isolate a lighting effect on a Sideways surface

Run a paired lighting comparison on a marked F12 capture with its PNG and
visibility attachments. Supply original PNG coordinates for older unmarked
captures. Add a nearby reference pixel to measure contrast across a stripe.

```sh
warp relight sideways-123.json --without gi --without shadow_strength
warp relight sideways-123.json --pixel 2300 540 --reference 2300 385 --radius 180 --without gi --out build/rock-light
```

Each `--without` produces a separate render with that control set to zero.
Choices are `gi`, `shadow_strength`, `near_shadows`, `contact_occlusion` and
`reflection_occlusion`. The tool needs Cargo, a render-capable GPU and the
developer checkout. Use a fresh output directory or use the path it prints.

Open `comparison.png` to see the original, the native baseline and each variant.
Orange marks the selected pixel and blue marks the optional reference when
there is room to draw the rings. Individual crops have no overlays.
In `lighting.json` check `source_triangle_matches_baseline` for each pixel,
`replay_triangle_stable` for each variant, and `repeat_crop_identical` for drift
when the baseline is rendered again. Compare the original and baseline
appearance before attributing an artifact to a control. Each sample includes
RGB, relative luminance in linearized display sRGB, and its delta from baseline.
With a reference, `reference_minus_point` measures the signed contrast.
Captured picks include hierarchy metadata when available. Full frames, camera,
surface hits and reconstruction details are in `native/`.

The replay reconstructs the selected viewport of a static stage at native
resolution with the captured camera, sun and wind. Gameplay upscaling, cars,
HUD and transient particles can change the original pixel. Biome stages
are currently unsupported.

### Replay a recorded drive

To replay a single build without comparing Git revisions, run from the repo root:

```sh
cargo run --locked -p cli --bin sideways -- replay-drive sideways-123.json build/drive-run --state-only
```

Use a fresh output path. In `drive.json`, `matches_capture` checks the final
physics digest. For two builds, read `baseline_matches_recording` first in
`comparison.json`. A false value means the baseline itself diverged from the
recorded endpoint. `first_difference_in_window` is null when the samples agree
through the comparison window.

Click the PNG in the inspector, or pass output-pixel coordinates from its top
left. The inspector maps the point through the captured split viewport and
internal resolution. For source triangles it reports the instance, mesh,
cluster LOD depth, material, shading path and triangle. New Sideways captures
also report `hierarchy` for a car body or wheel (car index, model, named part
and body instance), a rigid plant part (pose indices from the hit to its root),
or a baked animated part (actor and bone indices). These IDs belong to that
capture. Terrain chunks and other ungrouped meshes have no hierarchy field. It reads the opaque
visibility buffer of the capture frame. Transparent effects, HUD and temporal
reconstruction may cover that source. Sky, voxels, gaps and profiles without
source visibility have explicit results. Older PNG/JSON pairs lack this buffer.

Comparison builds Sideways in the baseline revision and working checkout and
runs them sequentially with identical stage-generation RNG state, biome,
settings and per-tick driver input. Both revisions need the replay reader.
It restores recorded gear mode and tree collider edits at their simulation
ticks. Input runs begin at stage creation; the final 600 ticks (five seconds)
supply the trace. Recording is bounded to one hour and 32,768 input runs per
stage. An incomplete history is marked unavailable.

The output contains synchronized 640 Ã— 360 chase-camera WebP clips (15 fps),
the final pixel difference, and `comparison.json` with the first differing
physics digest in the window, player values and whether each run reached the
recorded end digest. These diagnostic cameras and lighting are independent
of the play camera and HUD. Frame timing includes synchronous render/readback;
per-pass GPU timings and adapter are in each `drive.json`. The timing summary
excludes the first render. `--state-only` skips GPU work.
Use a fresh output directory. For prebuilt runners use `--baseline-exe`
and `--candidate-exe` with matching bundled assets. The same commands run via
`python engine/tools/drive-capture.py`. To generate a fixture without a visible
window, run `sideways play --smoke-render 76 --feedback --capture build/drift.png`.
