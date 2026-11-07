# Neon Trails

Leave a trail, leave no escape. Riders move one cell at a time on a 48 x 26
grid and can only turn; a wall, any trail or a head-on crash ends a rider's
round. The last rider standing scores 3, first to 15 wins.

A Luau game on the `warp` runtime, ported from the LÖVE party pack
(`ontola/gamenight-games`, `love-party/games/trails.lua`): the same grid,
step time, start positions, collisions, scoring and bot, in a 3D arena with
one shared camera (`split = false` in game.toml).

```sh
warp run . --bots 3                 # you and three bots (stick or arrow keys)
warp check . --ticks 3000           # rounds without a GPU
warp script . scripts/smoke.json --out /tmp/trails --bots 3
```

Not ported yet: the party settings (speed and round pause are constants at
the top of main.luau until Luau games can declare Gamepad Party settings).
