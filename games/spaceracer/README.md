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

Run `cargo test -p spaceracer --lib`. Five checks cover 4,800 source ground
ticks, 1,800 flight ticks, 700 drafting ticks, 1,680 bump ticks, physical gate
rejection and 18 source track fixtures (six layouts × three difficulties).
Track fixtures test sampling, not native procedural generation. Flight tests
cover the integrator, not takeoff/landing/collision integration. No full-game
performance result or visual-parity claim is available yet.

The offline tools export fixtures with the pinned Godot source checkout.
Fixtures are compressed with deterministic gzip metadata. The production
simulation crate has no Godot dependency.
