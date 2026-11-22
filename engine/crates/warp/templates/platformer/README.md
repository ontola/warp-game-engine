# Coin Climb

The platformer template: `warp new my-game --template platformer`. One to
four players run and double-jump up a tower of ledges for coins; first to
12, or the most after two minutes. Bots fill up to two players.

It is about 150 lines because the game kit does the rest
(`require("kit/...")`, see `KIT.md`): `controller.platformer` moves the
climbers, `camera` follows each one from the side, `round` runs the
countdown, clock and results, `ui` draws the scores and `bot` drives the
bots.

```sh
warp run .            # play (add --players 2 for split screen)
warp check .          # load and run 120 ticks without a GPU
warp script . scripts/smoke.json --out out   # the headless playtest
```

Things to change first: `WIN`, `LENGTH` and the level constants at the
top of `main.luau`, the options of `controller.platformer` (speed, jump,
air_jumps, gravity), and the colours.
