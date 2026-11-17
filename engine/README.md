# Warp engine

Reusable Rust systems for procedural outdoor worlds and couch games. Cargo
runs from the repository root. Games depend on the engine, never the
reverse: a game supplies its simulation, assets, tuning and integration.

| Crate | Responsibility |
| --- | --- |
| `kern` | Deterministic simulation: fixed ticks, state digests, divergence search, value traces |
| `render` | Cluster LOD, GPU visibility, shading, shadows, procedural terrain, vegetation, atmosphere, HUD and reconstruction |
| `app` | Game shell: window, fixed tick, input, split screen, headless frames, scripted playtests, F10/F11/F12 |
| `audio` | Sound synthesis, effects playback, the procedural synth and its surround mixer |
| `physics` | Rigid bodies on Rapier with enhanced determinism |
| `body` | Bodies that move without hand animation: standard skeleton, motion-captured clips, gaits, rigs |
| `assets` | glTF import with materials and textures; baked weighted characters |
| `architecture` | Buildings inside and out from glTF (AtomicBuildings, IFC): room light bake, room and outside shots |
| `net` | Online play over iroh: typed host/client messages |
| `gamenight` | Native Gamepad Party client: party host, seats, wire contract |
| `script` | Luau modules: sandbox, hot reload, deterministic step budget |
| `live` | Live control of a running game over MCP |
| `warp` | The `warp` executable for games made only of Luau, synth patches and assets |
| `xr` | OpenXR for the native Quest build |
| `webxr/` (JS) | Experimental standalone WebXR shell ([Quest demo](docs/quest-vr.md)) |

`assets/` holds licensed default fonts, reusable audio resources and renderer
test fixtures. `tools/` holds the Python diagnostics
([tools](tools/README.md), [frame budget](tools/frame-budget.md)).

```sh
cargo check --locked --workspace
cargo test --locked -p render --lib
cargo run --release -p app --example hello          # the smallest game
cargo run --release -p app --example terrain_fly    # procedural terrain flight
```

Start with the [documentation index](docs/README.md). [Making a
game](docs/making-a-game.md) walks through a Rust game, the
[`warp` executable](docs/warp-cli.md) a Luau one, and [status](docs/status.md)
says what works now and what is open. Dated notes and measurements are in the
[development log](docs/log/README.md).

Not built yet: a human editor UI, dynamic indoor GI and general character
animation beyond `body`'s clips and gaits.
