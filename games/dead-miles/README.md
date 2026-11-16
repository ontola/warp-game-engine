# Dead Miles (working name)

A co-op zombie road trip for one to four people on one screen. Each stop
is a generated street: search the houses and cars for food, fuel, medkits
and the very little ammo left, then get everyone back to the car before
the street fills up. Melee weapons wear out and break, guns are loud and
every shot draws the dead, and a director swings the street from quiet to
a siege and back. Down players can be revived; a dead player is a ghost
for a few seconds and then walks out of the car as a new survivor. The run
ends only when nobody is standing.

A Luau game on the `warp` runtime. The design is the "Zombie road-trip
co-op: design" doc; this is its first playable (one kind of stop, two
kinds of zombie, no strangers or events yet).

| | |
| --- | --- |
| Left stick / WASD | move |
| Right stick | aim (melee and guns follow it; else you face where you walk) |
| RT / left mouse (held) | fire the gun (shared ammo; aim help within about 20 degrees) |
| X, RB / R | swing the melee weapon |
| B / Shift | shove: pushes zombies back and breaks their wind-up |
| A / Space (hold) | search, pick up a weapon, revive, and at the car drive on |
| Y / F | use a medkit |
| As a ghost, A | moan: pulls nearby zombies towards you |

Every zombie attack is telegraphed: arms go up, the eyes light red and a
red ring shows its reach on the ground. Hitting or shoving it first
cancels the attack. Shamblers swipe; runners (from stop 2) lunge about
three metres.

```sh
warp run . --bots 3                 # you and three bots
warp check . --ticks 6000           # bots play without a GPU
warp script . scripts/smoke.json --out /tmp/dead-miles
cargo test --release -p warp --test game dead_miles
```

Set the `autopilot` knob to 1 to let bots drive the people too (demos,
store clips, long headless runs). Other tuning (speeds, starting stocks,
director pressure, peak and calm lengths, downed and ghost time, camera,
density) is in the `-- @knobs` block at the top of main.luau.

## How it is built

- `main.luau`: the rules. Survivors, zombies, the director, bots, the HUD.
- `lib/street.module.luau`: one generated street: road, houses with a
  front door, an inner wall and cupboards and closets, sheds, trees,
  parked cars (sometimes a police car), the group's car.
- `lib/figure.module.luau`: people built from boxes with arms and legs on
  pivots, a walk cycle and poses (reach, wind-up, swing, aim, shove, lying).

Collision is the game's own: circles against the street's boxes in a 4 m
grid, and zombies keep apart in a 1.5 m grid. Shots use `world.raycast`
against walls. Bots and zombies walk through front doors and the gap in
the inner wall (`waypoint`).

Cost: about 1.5 ms a tick on a cloud CPU with 160 zombies (mostly asleep)
and four survivors. Moving the horde to Rust is phase 2 of the design.

## Not yet

The route map, night camp, strangers and recruits, events, raiders, more
stop types and zombie kinds, phone screens, Gamepad Party settings
(the knobs stand in), animated `body` characters, the store clip.
