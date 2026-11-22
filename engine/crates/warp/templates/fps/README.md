# Frag Yard

The first-person template: `warp new my-game --template fps`. A
deathmatch for 1 to 4 players in split screen, bots in the empty seats;
first to 10 frags, or the most after three minutes.

About 250 lines on the game kit (`KIT.md`): `controller.first_person`
moves and looks, `camera`'s `first_person` looks through the eyes,
`weapon` is the rifle (rate, spread, ammo, reload, tracers) and the health,
`bot.shoot_at` plays the bots, `round` runs the match and `ui` draws the
crosshair, bars and names.

```sh
warp run .            # play (add --players 2 for split screen)
warp check .          # load and run 120 ticks without a GPU
warp script . scripts/smoke.json --out out   # the headless playtest
```

Change first: `weapon.new` options in `add_fighter` (a shotgun is
`pellets = 8, spread = 0.08, rate = 1.2`), `WIN` and `LENGTH`, the arena
in `build_arena`, and the bots' `skill`.
