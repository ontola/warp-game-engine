# Hello Warp

A Warp game made of Luau scripts and sound patches, run by the `warp`
executable without compiling anything. Capsules run and jump on a floor
and collect glowing orbs (with a pickup sound); first to 10 wins. Each
person gets a split-screen view, bots chase the nearest orb, and under
Splitscreen Club the party's seats are the players. Start opens the menu.

```sh
warp run .                      # play in a window (WASD, mouse, Space, Enter for the menu)
warp run . --players 2 --bots 2 # two people in split screen and two bots
warp run . --live               # ... and let an AI change it over MCP
warp check .                    # init and 120 ticks without a GPU; prints the state
warp script . scripts/smoke.json --out /tmp/hello   # headless playtest with frames
warp audio render sounds/pickup.luau                # measure a patch without speakers
```

- `main.luau`: `init()`, `tick(dt, input)`, `draw()` and `reload()`.
  Save it while the game runs: it reloads, and the world and `state` stay.
- `sounds/*.luau`: synth patches; `sound.play("pickup")` plays
  `sounds/pickup.luau`. They reload when saved too.
- `assets/`: glTF models (`world.spawn("assets/crate.glb")`).
- `types.d.luau`: the whole API, for reading and for editor completion.

The guide is engine/docs/warp-cli.md in the Warp repository.
