# The game kit

The kit is a set of Luau libraries built into the `warp` executable. Any
game can load them with `require`, also a one-file game, with nothing to
copy:

```lua
local controller = require("kit/controller")
local cam = require("kit/camera")
local ui = require("kit/ui")
local round = require("kit/round")
local bot = require("kit/bot")
local timer = require("kit/timer")
local util = require("kit/util")
local weapon = require("kit/weapon")
```

Use the kit before writing your own movement, cameras, HUD or match flow.
It is tested, it keeps games deterministic (game time, the seeded
`random`), and it gives every game the same feel on Splitscreen Club. To
change a library, copy it from `engine/crates/warp/kit/NAME.luau` to
`kit/NAME.module.luau` in your game folder: a game's own copy wins.

Kit objects (controllers, rigs, rounds, timers) are tables with methods.
Keep them in `state`, so they survive hot reloads, and call methods with
`:` (`state.round:tick(dt)`).

`warp new my-game --template platformer` (or `fps`) starts from a whole
game built on the kit (`warp templates` lists them).

## A whole game, short

```lua
local controller = require("kit/controller")
local cam = require("kit/camera")
local ui = require("kit/ui")
local round = require("kit/round")
local bot = require("kit/bot")
local util = require("kit/util")
local v = vector.create

local function init()
	sky.set("afternoon")
	world.spawn("cube", { pos = v(0, -0.5, 0), size = v(40, 1, 40), color = "#6f9a5a" })
	state.round = round.new({ win_score = 5, length = 90 })
	state.heroes = {}
	for _, p in players.list() do
		local body = world.spawn("capsule", { pos = v(p.id * 2, 1, 0), size = v(0.8, 0.9, 0.8), color = p.color, solid = false })
		state.heroes[p.id] = { body = body, bot = p.bot, ctl = controller.walker(body, { dash = 14 }), cam = cam.rig(p.id) }
	end
end

local function tick(dt, input)
	state.round:tick(dt)
	for id, h in state.heroes do
		local pos = world.get(h.body).pos
		local inp = if h.bot then bot.walk_to(pos, v(0, 0, 0)) else input[id]
		if not state.round:playing() then inp = bot.input() end
		h.ctl:update(inp, dt)
		if not h.bot then h.cam:chase(pos, h.ctl.yaw, dt) end
	end
end

local function draw()
	for id, h in state.heroes do
		local p = players.get(id)
		if p and not h.bot then ui.score(id, p.name, state.round:score(id), p.color) end
	end
	state.round:draw()
end

return { init = init, tick = tick, draw = draw }
```

## kit/controller

Movement for a body (an entity from `world.spawn`), one player's input per
tick. Collision is `world.move`: the body slides along solid boxes as an
upright cylinder of its box's width and height. Make player bodies
`solid = false` so players pass through each other (they still stand on
and bump into the level).

| Constructor | For | Options (defaults) |
| --- | --- | --- |
| `controller.platformer(body, opts)` | side view in the X-Y plane: run with move_x, jump with A/Space, hold for higher, coyote time and jump buffer | `speed` 7, `accel` 70, `air_accel` 40, `friction` 60, `jump` 13 (about 2.6 m), `gravity` 32, `fall_gravity` 55, `max_fall` 28, `coyote` 0.1, `buffer` 0.12, `air_jumps` 0, `depth` (the plane's z: the spawn's; false: free) |
| `controller.walker(body, opts)` | top-down, twin-stick, arena, party: the stick walks on the ground relative to a view yaw, faces where it walks or aims | `speed` 6, `accel` 45, `friction` 40, `turn` 14, `jump` 0 (none), `gravity` 28, `face` "move" or "aim", `view_yaw` 0, `dash` 0 (speed; B dashes), `dash_time` 0.15, `dash_cooldown` 0.7 |
| `controller.third_person(body, opts)` | look_x turns, the stick walks relative to facing, LB sprints | `speed` 6, `sprint` 1.6, `accel` 50, `jump` 7.5, `gravity` 22 |
| `controller.first_person(body, opts)` | shooters: a third_person tuned for a camera in the head; `ctl:eye()` is the eye point, `ctl:aim()` the look direction | `speed` 7, `sprint` 1.35, `accel` 70, `jump` 6.5, `gravity` 20, `eye_height` 0.7, `max_pitch` 1.45 |
| `controller.kart(body, opts)` | arcade driving: stick forward or RT throttle, back or LT brake and reverse, sideways steers | `max_speed` 24, `accel` 16, `brake` 35, `reverse` 7, `drag` 4, `turn` 2.4, `grip` 8 (below 4 drifts), `gravity` 25 |

All take `spawn` (where to respawn; default where the body is),
`kill_y` (-30: below it the body respawns) and `query` (a `world.move`
query, `{ tag, ignore }`).

`ctl:update(input, dt[, view_yaw])` moves the body and returns this
tick's events: `jumped`, `landed`, `fell` (respawned), and `air_jumped`
(platformer), `dashed` (walker), `bumped` (kart hit a wall fast). Fields:
`vel`, `grounded`, `yaw`; `facing` (platformer, 1 or -1), `pitch` (third
person), `speed` (kart). Extras: `platformer:bounce(speed?)`,
`walker:push(impulse)`, `walker:stick(input)`, `kart:boost(seconds,
extra?)`, `kart:speed01()`.

## kit/crowd

`require("kit/crowd").new(pool, radius)` shares a fitted Warp Humans pool among cameras. Give each candidate a stable positive integer ID, feet `pos`, human `yaw`, and optional `active` and `pose`. IDs can have gaps as areas unload. Call `crowd:select(candidates, observerPositions)` periodically to allocate the nearest people in turns across observers. An optional third argument `rank(candidate, observerIndex, squaredDistance)` prioritizes candidates per view. Return a lower number for a higher priority or `nil` to exclude a person from that viewâ€™s selection. Call `crowd:update(candidates)` each tick to animate the selected people. The game owns their routes and schedules.

A third argument to `crowd.new(pool, radius, seed)` assigns stable appearances. Set `distant` in the pool's TOML to reserve low-detail residents. At each selection, call `humans.distant(pool, streetRows, crowd.active, seconds)` with a dense array of candidates, their next feet position in `nextPos`, and their next heading in `nextYaw`. The renderer interpolates one shared set of unskinned bodies between these snapshots. A fitted resident keeps the same body dimensions, garment dyes, skin and hair. `crowd:promote(id, candidates, observers)` fits a resident for an immediate interaction. Set `externalPose` while another system drives that actor's pose.

## kit/daycycle

`require("kit/daycycle").new(startHour, secondsPerDay, weather)` creates a game-time clock. Call `daycycle.tick(clock, dt)` once per simulation tick. All views share its `hour`, `phase` and `daylight`. It switches between six sky presets and warms the next atmospheric bake before the transition.

## kit/camera

```lua
state.cams[p.id] = cam.rig(p.id, { fov = 60, smooth = 7 })   -- one per person
rig:chase(pos, yaw, dt, { distance = 7, height = 3, look_height = 1, pitch = 0 })
rig:side(pos, dt, { distance = 16, height = 3, look_height = 1.5, vel = ctl.vel, lead = 2 })
rig:top_down(pos, dt, { height = 16, back = 7 })
rig:frame(points, dt, { pitch = 0.95, yaw = 0, margin = 1.3, min_distance = 10, max_distance = 80 })
rig:first_person(ctl:eye(), ctl.yaw, ctl.pitch)        -- no easing
rig:look(pos, target, dt, fov?)   rig:snap()   rig:shake(strength?)
```

`cam.rig()` without a player is the first person's camera: the one shared
view of a game with `split = false`, framed on everyone with `frame`. Move
cameras in `tick` (sound listeners follow them). The first call snaps;
`snap()` makes the next one snap again (after a respawn).

## kit/ui

Call from `draw`. Sizes are points (pixels at 720p, scaled with the
screen); `player` puts a piece in that person's split-screen view, nil
uses the whole screen.

| Function | Draws |
| --- | --- |
| `ui.score(player, name, score, color?)` | name and score, top left of the view, with a colour stripe |
| `ui.bar(player, label, frac, { color, row, width })` | a labelled bar under the score (health, boost) |
| `ui.banner(text, { sub, player, size = 72, color, y = 0.38 })` | big centred text |
| `ui.countdown(seconds_left, player?)` | 3, 2, 1, GO! |
| `ui.clock(seconds, player?)` | 1:05 at the top centre |
| `ui.scoreboard(rows, { title, player })` | a results table; rows `{ name, score, color }` |
| `ui.marker(pos, text, { player, color, size, lift })` | a label over a world point |
| `ui.crosshair(player, { color, size, spread })` | a crosshair in the middle of the view |
| `ui.flash(player, color)` | tints the view (pass a fading alpha: hit flash) |
| `ui.hint(text, player?)` | a line of help at the bottom |
| `ui.text(text, x, y, { size, color, align, width, shadow })`, `ui.panel(x, y, w, h, color?)` | the pieces the others use (pixels) |
| `ui.scale()`, `ui.area(player?)` | pixels per point; a view's x, y, w, h |

## kit/round

The flow of a couch match: countdown, play to a score or a time, results,
then `game.finish()` so Splitscreen Club moves on.

```lua
state.round = round.new({ countdown = 3, length = 120, win_score = 10, results = 6, title = "..." })
state.round:tick(dt)                 -- first thing in tick
state.round:playing()                -- false during countdown and results
state.round:add(id, points?)         -- may end the round at win_score
state.round:score(id)  :set(id, n)  :standings()  :time_left()
state.round:finish(winner?)          -- an id, a list, or nil for the top score
state.round:join(id)                 -- in player_joined
state.round:draw()                   -- countdown, clock, GO!, results
```

Phases (`state.round.phase`): `countdown`, `playing`, `results`, `done`.
`length` nil plays until `win_score`; `win_score` nil plays for `length`.

## kit/bot

Inputs for bots, shaped like a person's, so the same controller drives
both.

| Function | Gives |
| --- | --- |
| `bot.input()` | a neutral input |
| `bot.press(inp, button)` | presses a button this tick (`jump`, `fire`, `east` ...) |
| `bot.walk_to(from, to, { view_yaw, slow, skill })` | a walker's stick towards a point |
| `bot.aim_at(inp, from, to, view_yaw?)` | twin-stick aim at a point |
| `bot.run_to(body, to, { skill, ctl })` | a platformer's run and jumps towards a point; finds openings under ledges, unsticks itself; pass `ctl` to double-jump |
| `bot.shoot_at(ctl, target, { skill, range, ignore })` | a first-person bot's input: turns and aims with skill-scaled wobble, keeps range and strafes, fires after a reaction time when the line is clear |
| `bot.drive_to(from, yaw, to, { skill })` | a kart's steering and throttle towards a point |

## kit/timer

```lua
state.timers = timer.new()
state.timers:after(2, fn)  :every(0.5, fn, count?)  :cancel(id)  :left(id)  :clear()
state.timers:tick(dt)                -- once per tick
state.shot = timer.cooldown(0.25)    -- :use() true at most every 0.25 s; :ready() :progress() :reset()
local t = timer.tween(from, to, seconds, timer.ease.out_back)   -- :value() :done() :progress()
```

Easing: `linear`, `smooth`, `in_quad`, `out_quad`, `in_out_quad`,
`in_cubic`, `out_cubic`, `in_out_cubic`, `out_back`, `out_elastic`,
`out_bounce`, `pulse` (0 to 1 to 0). Everything runs on game time.

## kit/weapon

Hitscan guns and health for shooters. Hits are `world.raycast`, so targets
must be `solid` bodies (give them a tag and pass `{ tag = ... }`).

```lua
state.gun = weapon.new({ damage = 25, rate = 8, auto = true, spread = 0.01, pellets = 1, range = 120, ammo = 24, reload = 1.4 })
local shot = gun:fire(input, ctl:eye(), ctl:aim(), { tag = "fighter", ignore = body })
-- nil, or { from, dir, hits, ends }: each hit is a world.raycast hit plus damage; west (X) reloads
gun:tick(dt)   gun:reload()   gun:reloading()   gun:ammo_text()   gun.clip   gun.kick
weapon.tracer(from, to, color?)   -- a short-lived streak; call weapon.update() once per tick
weapon.warm()                      -- in init, so the first shot doesn't hitch
state.hp = weapon.health(100)      -- :hurt(n) true on death, :heal(n), :reset(), :alive(), :frac(), :since_hurt()
```

`warp new my-game --template fps` is a whole deathmatch with these: first
person split screen, bots, respawns, spawn protection, a gun in your hands.

## kit/util

`forward(yaw)`, `right(yaw)`, `yaw_to(dir)`, `angle_diff(a, b)`,
`turn_towards(yaw, target, step)`, `flat(v)`, `with_y(v, y)`,
`lerp(a, b, t)`, `approach(x, target, step)`, `damp(x, target, rate, dt)`,
`remap(x, a, b, c, d)`, `distance(a, b)`, `flat_distance(a, b)`,
`clamp_length(v, max)`, `nearest(pos, tag, skip?)` (id, distance),
`random_in_circle(centre, r)`, `shuffle(list)`, `pick(list)`,
`rgba(color, alpha)`, and `ZERO`, `UP`. Yaw 0 faces -Z; +Y is up.

## Measuring the kit

`engine/tools/vibe-eval/vibe-eval.py` asks a model to make a game for each
prompt in `prompts.json` (25 couch-game ideas across genres) with only
these docs, then scores every game headless with `warp check` and three
bots: it loads, runs 30 s without errors, its state changes, and a round
ends within 3 minutes. Run it after changing the kit or its docs and
compare the totals; `vibe-eval.py score GAME...` scores games you already
have.

```sh
python3 engine/tools/vibe-eval/vibe-eval.py make --only sumo,tag \
  --agent 'claude -p "$(cat {prompt})" --model sonnet --permission-mode acceptEdits'
```

### Local vehicle damage

`require("kit/car_damage")` models front, rear, left and right zones in vehicle-local metres (forward −Z). Define the half width, distances to both bumpers, and the three inner crush boundaries with `damage.profile(.98, 2.21, 2.19, .48, 1.04, 1.16)`. Keep one plain `damage.new()` state per vehicle. `damage.hit(state, profile, localPoint, severity)` accumulates a normalized impact. Corner hits affect two zones. `damage.controls(state, controls, signedSpeed)` applies the resulting loss of power and steering.

For smooth dents, call `damage.dent(state, profile, localPoint, severity)` at the hit location. It merges nearby strikes and retains up to eight local fields. Declare compatible static glTF paths together at startup using `world.warm_dents(paths, slots)`, with one to eight slots. Use `world.dent(body, state.dents, missingMask)` to assign a slot or update the surface. The return is false when every slot is occupied. `world.dent(body, nil, missingMask)` releases the slot. Each car keeps its own damage history. The renderer updates positions and normals on impacts or slot reassignment, reusing the original paint, UVs and triangles at every detail level.

To detach panels, assign unique node names and integer `warpPart` extras (0 for the frame, 1–31 for removable IDs, up to six detachable groups per model). A missing part sets bit `1 << id`. A detached model is available as `assets/model.gltf#part-name`. `world.part_center(path)` returns its original position in the body. Set the boolean primitive extra `warpDetachedOnly` to true for lining that should appear only on the detached part. Unannotated models support smooth dents. The older `warpCrushGrid` and `damage.cellPose` APIs support authored section scaling.

Warp Theft Auto gives the eight nearest damaged cars within 48 metres of any player the detailed mesh. Additional cars retain their damage state and show missing panels at a coarse level. `vehicles.dent_query(id, profileName, fields, body, missingMask)` refits the gameplay query for rays and splash after a hit. Removed panels open the same gaps in the query mesh. The road solver and pedestrians use the stable driving chassis. `vehicles.impacts()` reports each car's strongest hard contact during the latest simulation step, including the world position and normal speed change (m/s).
