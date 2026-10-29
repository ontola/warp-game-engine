# Space Racer on Warp

Faithful port of joepio/spaceracer, pinned in port-status.json. Work in progress:
not yet a playable or benchmark-equivalent replacement. Never report a faster
engine result by silently omitting source gameplay, shaders, scenery or effects.

Preserve 120 Hz simulation, all six layout recipes, five biomes, three difficulties,
1-4 local views, AI, flight/landing, weapons, recharge, checkpoints, results,
menus, audio and GameNight behavior. Original source remains external; Godot is
used only to generate test oracles and export source assets during development.
The Warp executable must not run Godot or communicate with a Godot simulation.

First acceptance gate: identical seeded track samples and scripted control traces.
Then asset/material/effect parity and synchronized camera image comparisons.
Only then benchmark equal resolution, seats, bots, seed, effects and scripted
inputs, with independent CPU/GPU timing and repeated warm runs. Startup/build
costs are reported separately. Source attribution: licenses/SpaceRacer-MIT.txt.

## Current verification

Run `cargo test -p spaceracer --lib`. Ten checks cover 4,800 source ground
ticks, 1,800 flight ticks, 700 drafting ticks, 1,680 bump ticks, physical gate
rejection and 18 source track fixtures (six layouts × three difficulties).
Native generation now also matches all nodes and gameplay features in the 18
reference ribbons within 3 cm positional tolerance. Base and raw centrelines
are checked across 90 layout/difficulty/biome combinations. Flight tests
cover the integrator, not takeoff/landing/collision integration. No full-game
performance result or visual-parity claim is available yet.

The offline tools export fixtures with the pinned Godot source checkout.
Fixtures are compressed with deterministic gzip metadata. The production
simulation crate has no Godot dependency.

Camera chase is checked for 1,800 ticks across three viewport aspect ratios,
including roll and boost shake. Five landing scenarios compare capture, damage,
crashes and underside rejection with the source. The renderer has opt-in
per-view up axes (`Renderer::set_camera_up_axes`) for loops and free flight.

`assets/ships.json.gz` preserves five authored variants, 218 meshes, 145 material
records, control pivots, shader parameters and optical effect geometry. This is
asset extraction, not proof that the native renderer already implements these
materials. `assets/manifest.json` records the hashes of 102 unchanged original
asset files. The source shaders are reference inputs for the material port;
Warp does not execute Godot shader source.

Remaining integration is explicitly tracked in `port-status.json`: full race
lifecycle, contacts, AI/weapons, all biome scenery, shaders/effects, menus/HUD,
audio, GameNight and controller bindings. There is intentionally no play or
benchmark launcher advertised until those parity gates are met.
