# Ricochet Club

Every wall is another angle. Tanks fire shots that bounce off the arena
walls (four bounces, then they fade) and chip away at wooden cover: three
hits break a crate. Hitting another tank scores 3 and costs them 1; your own
shot can hit you once it has bounced. A hit tank cannot be hit again for
1.2 s. The highest score after 60 seconds wins the round, and the next
round starts three seconds later.

A Luau game on the `warp` runtime, ported from the LÖVE party pack
(`ontola/gamenight-games`, `love-party/games/ricochet.lua` and
`shared/arena.lua`): the same arena units, spawns, cover, speeds, fire and
shield timings, collisions, scoring, round length and bot, stepped at
120 Hz as there. The simulation runs in the LÖVE 2D units; `sync()` shows
it in 3D with one shared camera (`split = false` in game.toml), and the
engine draws the debris and shield flashes (`fx.burst`, `fx.ring`).

| | |
| --- | --- |
| Left stick / WASD / d-pad | drive |
| Right stick | aim (without it, a tank aims where it drives) |
| RB, RT / Space, E, Shift, left mouse | fire (every 0.45 s) |
| B, Y / right mouse, F | shield for 0.6 s (every 3 s) |

```sh
warp run . --bots 3                 # you and three bots
warp check . --ticks 3000 --bots 3  # most of a round without a GPU
warp script . scripts/smoke.json --out /tmp/ricochet --bots 3
```

Sounds (`sounds/`) are synth patches: fire, bounce, hit (cover or a
shield), shield, explode (a tank hit or a crate breaking), point and round.

Not ported yet: the party settings (bounces, shot speed and cover are
constants at the top of main.luau until Luau games can declare GameNight
settings). Up to four tanks play, as in the LÖVE pack; further players
watch.
