# Anticonception

Protect the egg. A co-op game for one to four: every guard is an arc on a
ring around the egg. Swimmers come in from the edge, more and faster as
time goes on; a guard that catches one scores 10 for the team, and every one
that gets through costs the egg one of its five lives. Survive 90 seconds.

A one-file Luau game on the `warp` runtime (`anticonception.luau`: settings
in its `--!` lines, synth sounds inline), ported from the LÖVE party pack
(`ontola/gamenight-games`, `love-party/games/orbit.lua`, id `orbit-guard`):
the same radii, guard widths, turn and boost speeds, wide-guard timing and
cooldown, the 3 s grace, spawn interval, swimmer speeds, lives, scoring,
waves and bot, in 3D with one shared angled camera.

| | |
| --- | --- |
| Left stick / A, D / d-pad | turn around the egg |
| A, RB / Space (held) | turn faster |
| B | wide guard for 1 s, every 4 s |

```sh
warp run anticonception.luau --bots 3    # you and three bots
warp check anticonception.luau --ticks 3000
warp script anticonception.luau scripts/smoke.json --out /tmp/egg --bots 3
```

The bot is LÖVE's (turn to the nearest swimmer outside the ring, boost
when it is far, widen when it is close) with one change: it leaves a
swimmer to a guard that is closer to it. In LÖVE, bots that chase the same
swimmer stay stacked on one spot for the rest of the round.

Not ported: LÖVE's automatic rematch after the results (the round ends with
`game.finish()`; New round in the start menu plays again).

The folder is a game too (`warp run games/anticonception`): its only script is the
game, so GameNight's Windows packaging (`engine/crates/warp/tools/package-windows.ps1`)
packages it as it is.
