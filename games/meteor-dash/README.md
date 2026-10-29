# Meteor Dash

Chase the stars, dodge everything else. Stars and meteors stream down the
field; a star is +5, a meteor -10 (followed by 0.75 s of shield), and every
second alive is a point. A shield dash gives 0.3 s of shield at more than
double speed, every 1.5 s. Most points after 60 seconds wins.

A one-file Luau game on the `warp` runtime (`meteor-dash.luau`: settings in
its `--!` lines, synth sounds inline), ported from the LÖVE party pack
(`ontola/gamenight-games`, `love-party/games/meteor.lua`): the same field,
spawn rate, object sizes and speeds, shield timings, scoring, round length
and bot, in 3D with one shared angled camera.

| | |
| --- | --- |
| Left stick / WASD / d-pad | fly |
| A, RB / Space | shield dash |

```sh
warp run meteor-dash.luau --bots 3       # you and three bots
warp check meteor-dash.luau --ticks 3000
warp script meteor-dash.luau scripts/smoke.json --out /tmp/meteor --bots 3
```

The LÖVE bot is ported as is: it drifts side to side, steps towards a star
above it and away from a meteor, and never dashes.

Not ported: LÖVE's automatic rematch after the results (the round ends with
`game.finish()`; New round in the start menu plays again). GameNight's
Windows packaging (`engine/crates/warp/tools/package-windows.ps1`) still
expects a `main.luau`: rename or copy `meteor-dash.luau` to `main.luau` to
package it.
