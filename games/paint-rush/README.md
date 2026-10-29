# Paint Rush

Leave your colour everywhere. Every tile you roll over turns your colour,
and a burst paints a diamond around you every four seconds. Two players
claiming the same tile in the same step leave it as it was. Your score is
the number of tiles you own. After 60 seconds, the most tiles wins.

A Luau game on the `warp` runtime, ported from the LÖVE party pack
(`ontola/gamenight-games`, `love-party/games/paint.lua` and
`shared/arena.lua`). The 37 x 16 field, start positions, speed, burst
shape, cooldown, contested tiles, scoring and bot are the same, and the
simulation runs at LÖVE's 120 Hz. In 3D, floor tiles pop up as they change
colour under one shared camera (`split = false` in game.toml). A bar at the
bottom shows who owns how much of the floor.

| | |
| --- | --- |
| Left stick / WASD | roll |
| A, RB / Space | burst (every 4 s; a ring under you means it is ready) |

```sh
warp run . --bots 3                 # you and three bots
warp check . --ticks 3000 --bots 3  # a round without a GPU
warp script . scripts/smoke.json --out /tmp/paint --bots 3
```

Each round ends with `game.finish()`, the way the LÖVE runner reports
every round. Without a party host the next round starts after three
seconds.

Not ported yet: the party settings. The round length is a constant at the
top of main.luau until Luau games can declare GameNight settings.
