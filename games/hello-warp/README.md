# Hello Warp

A Warp game made of Luau scripts and sound patches, run by the `warp`
executable without compiling anything. A capsule runs and jumps on a
floor, collects glowing orbs (with a pickup sound) and shows the score.

```sh
warp run .                      # play in a window (WASD, mouse, Space)
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
