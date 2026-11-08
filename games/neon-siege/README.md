# Neon Siege

One swarm, one team, keep moving. A co-op twin-stick shooter over 24
waves: a boss every fourth wave (Overseer, Hive Queen, Serpent, Prism
Twins, Solar Warden) and the Singularity at the end. From wave 7 a black
hole opens on every other wave; it pulls everything in and its core kills.

A Luau game on the `warp` runtime, ported from the LÖVE party pack
(`ontola/gamenight-games`, `love-party/games/siege*.lua`): the same arena
units, enemies, waves, bosses, beams, black hole, power-ups (spread,
pierce, rapid, life), revive, scoring and bot. The simulation runs in the
LÖVE 2D units; `sync()` shows it in 3D with one shared camera, and the
engine draws the bursts and rings (`fx.burst`, `fx.ring`).

| | |
| --- | --- |
| Left stick / WASD | fly |
| Right stick | aim and fire |
| RT / left mouse (held) | fire at the nearest enemy |
| A, RB / Space | dash (dodges hits) |
| B | pulse: clears small enemies and bullets nearby, every 10 s |
| Stand next to a downed pilot | revive them (or wait six seconds) |

```sh
warp run . --bots 3                 # you and three bots
warp check . --ticks 6000 --bots 3  # a few waves without a GPU
warp script . scripts/smoke.json --out /tmp/siege --bots 3
```

Not ported yet: the party settings (difficulty, starting wave, lives and
gravity are constants at the top of main.luau until Luau games can declare
Gamepad Party settings), the nebula sky and the warped grid around the black
hole.
