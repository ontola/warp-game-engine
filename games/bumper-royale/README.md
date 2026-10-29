# Bumper Royale

Own the ring, bump your friends out. Pucks slide around a round platform
that shrinks for the first minute. Ride off the edge and you lose a point.
Whoever bumped you in the three seconds before scores 3. A dash gives a
burst of speed every 1.1 s. A round lasts 60 seconds and the highest score
wins it.

A Luau game on the `warp` runtime, ported from the LÖVE party pack
(`ontola/gamenight-games`, `love-party/games/bumper.lua`). The arena units,
acceleration, friction, dash, bump impulse, shrinking ring, respawn ghost,
scoring and bot are the same, and the simulation runs at LÖVE's 120 Hz.
In 3D the pucks sit on a floating platform with a glowing rim under one
shared camera (`split = false` in game.toml). A puck that leaves the ring
tumbles off the edge.

| | |
| --- | --- |
| Left stick / WASD | slide |
| A, RB / Space | dash (in the direction you steer) |

```sh
warp run . --bots 3                 # you and three bots
warp check . --ticks 3000 --bots 3  # a round without a GPU
warp script . scripts/smoke.json --out /tmp/bumper --bots 3
```

Each round ends with `game.finish()`, the way the LÖVE runner reports
every round. Without a party host the next round starts after three
seconds. Scores start at zero every round.

Not ported yet: the party settings. The round length is a constant at the
top of main.luau until Luau games can declare GameNight settings.
