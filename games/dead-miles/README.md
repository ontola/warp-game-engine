# Dead Miles (working name)

A co-op zombie walk for one to four people on one screen. The group walks
east through a land that changes every day (suburb, forest, ruined city,
farmland) and decides which buildings to enter. Buildings have rooms
behind doors, with cupboards, closets and shelves to search, and
sometimes somebody inside. The dead gather behind you, so lingering
costs. Melee weapons wear out and break, there is almost no ammo, guns
are loud, and the group walks by night, because by day the dead see
further and move faster: each leg starts at dusk and ends with a dash
through the first daylight to a shelter. Torches eat batteries, and a lit
torch is seen from further away.

A hit leaves a survivor injured (slower, until a first aid kit), a second
hit puts them down. Down players can be revived and stay injured. A dead
player is a ghost for a few seconds, then walks back in as a new
survivor, or takes over a stranger who joined the group. At the end of a
day the group reaches a camp fire: everyone eats, the downed get up, and
the next day starts. The run ends only when nobody is standing.

A Luau game on the `warp` runtime. The design is the "Zombie road-trip
co-op: design" doc.

| | |
| --- | --- |
| Left stick / WASD | move |
| LT / right mouse (held) | run: faster, but the dead hear your footsteps |
| Right stick | aim (melee and guns follow it; else you face where you walk) |
| RT / left mouse (also X, RB, R) | attack with what is in your hand: fire the gun (shared ammo; aim help within about 20 degrees), or swing (costs stamina: the bar under you; heavy weapons cost more, some people have more) |
| A / Space (hold) | search, pick up a weapon, revive, talk to a stranger |
| Y / F | use a first aid kit |
| LB / Q | torch on or off |
| D-pad up and down, A, B | pick an answer when talking, confirm, walk away |
| As a ghost, A | moan: pulls nearby zombies towards you |

Every zombie attack is telegraphed: the arms go up for a second and the
eyes light red (no ring on the ground: the body shows it). Hitting it first
cancels the attack. Shamblers swipe; runners lunge about three metres.

```sh
warp run . --bots 3                 # you and three bots
warp check . --ticks 6000           # bots play without a GPU
warp script . scripts/smoke.json --out /tmp/dead-miles
cargo test --release -p warp --test game dead_miles
```

The dead come to sound. A gunshot carries 25 to 35 metres, rummaging
through a car about 9, running about 6, a swing a few. Enough
noise and a horde comes for the last loud thing it heard; keep quiet and
it settles. There is no clock that sends them otherwise: stragglers
wander in, and they head for a noise when there was one.

Set the `autopilot` knob to 1 to let bots drive the people too (demos,
store clips, long headless runs). Other tuning (speeds, limp, starting
stocks, battery drain, encounter chance, director pressure, downed and
ghost time, camera, density) is in the `-- @knobs` block at the top of
main.luau.

## Writing an encounter

Every file in `content/encounters/` is one encounter; the game loads them
all with `libraries("content/encounters")`, so a new file is a new
encounter. A file returns a table:

```lua
return {
	id = "trader",
	title = "Trader",                 -- shown on the talk card
	says = "Easy. I trade, I don't fight.",
	where = "room",                   -- "room" (inside a building), "path" or "camp"
	biomes = { "suburb", "city" },    -- optional: only in these biomes
	min_day = 2,                      -- optional: not before this day
	weight = 3,                       -- optional: how often, against the others (1)
	look = { shirt = "#5a6a4a" },     -- optional: shirt, skin, pants colours
	held = "pipe",                    -- optional: what they carry (and fight with if they join)
	temperament = { honest = 4, thief = 1 }, -- optional: a hidden roll, once per stranger
	choices = {
		{
			label = "2 food for 3 rounds",
			cost = { food = 2 },          -- greyed out answer if the group cannot pay
			results = {                   -- one is picked by weight, among those whose
				{ gain = { ammo = 3 }, text = "+3 ammo" }, -- temperament matches (or has none)
			},
			stay = true,                  -- optional: they stay to talk again
		},
	},
	on_choice = function(stranger, choice, result) end, -- optional hook
}
```

Items are `food`, `ammo`, `meds` (first aid kits) and `battery` (percent).
A result can have `gain`, `lose`, `weapon` (drops a pipe, bat, machete or
axe), `noise` (metres the dead hear it), `injure`, `heal` (the whole
group), `join` (they become a survivor bots drive, and a body for a dead
player), `hostile` (they shove and run) and `text`.

## How it is built

- `main.luau`: the rules. Survivors, health, torches, zombies, the
  director, encounters, recruits, bots, the camp, the HUD.
- `lib/land.module.luau`: the land in 24 m chunks, seven a day and a camp.
  Each chunk has a biome, a path, closed buildings (solid, with roofs,
  boarded doors; they hide when they would block the view of a survivor),
  a few open ruins made of a grid of rooms with
  doors (a spanning tree plus a few extra doors), containers in the rooms,
  trees, cars, street lights and weapons.
- `lib/body.module.luau`: a body for anyone, a Warp Human from one of
  game.toml's `[humans.*]` pools when one is free, else a box figure. The
  dead in view, survivors and strangers get humans; far zombies are boxes
  and swap to humans as they come near.
- `lib/figure.module.luau`: people built from boxes with arms and legs on
  pivots, a walk cycle and poses (reach, wind-up, swing, aim, shove, lying).
- `lib/mat.module.luau` and `surface.wgsl`: materials. Everything is a box
  or a ball; its colour picks a material kind (`mat.BY_COLOR`), and
  `surface.wgsl` draws it as asphalt, paving, grass, brick, plaster,
  concrete, wood, metal, rust, tar roof, roof tiles, leaves, cloth, rubber,
  glass, car paint, logs or bark around that colour, with a bump from its
  height. The land and the props spawn and warm through `mat.spawn` and
  `mat.warm`, so a new colour needs an entry there (and a warm).
  `debug.go("car")` puts the group by the next car, to look at things.
- `content/encounters/`: who you meet.

Collision is the game's own: circles against the land's boxes in a 4 m
grid, and zombies keep apart in a 1.5 m grid. Shots use `world.raycast`
against walls. Bots and zombies route through doors with a search over
each building's rooms (`waypoint`). Chunks stream in ahead and are
dropped behind; the left edge follows the group 13 m back.

## Not yet

More encounters, more zombie kinds, raiders, phone screens, Gamepad Party settings (the knobs stand in), the store
clip.
