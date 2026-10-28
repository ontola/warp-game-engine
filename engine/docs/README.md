# Warp documentation

Warp is a Rust game engine designed for fast iteration with AI. Everything
is code and text, there is no visual editor, every run replays bit for bit,
and a running game can be inspected and changed by an AI over MCP. It is
made for couch games with up to four players on one screen. Its focus so
far is procedural outdoor worlds, with headless rendering, scripted
playtests and hot-reloaded Luau.

Games on it: [Sideways](../../games/sideways/README.md), a gravel rally
game, and [Growing Guns](../../games/growing-guns/README.md), an arena
shooter. Spellstorm, a co-op wizard roguelite, and GameNight's Game Room
lobby are being ported. The `warp` executable runs games made only of Luau
scripts and sound patches, such as
[Hello Warp](../../games/hello-warp/README.md), with no Rust.

Start with the overview, then build and run something.

## Guides

| Guide | What it covers |
| --- | --- |
| [What Warp is](overview.md) | Design goals: deterministic sim, fast iteration, headless runs, physical units, live tweaking by an AI as a direction. |
| [Workspace and crates](workspace.md) | The repository layout, the eight engine crates, the game packages and the build profiles. |
| [Building and running](building-and-running.md) | Checks and tests, running the games, software rendering, scripted playtests, golden images, environment variables. |
| [Games in Luau: the warp executable](warp-cli.md) | `warp new`, `run`, `script`, `serve`, `check`: a game of Luau scripts, sound patches and glTF assets with hot reload and live MCP control. The Luau game API. |
| [Making a game](making-a-game.md) | The `app::Game` trait, the loop, building a scene, costs and gotchas. Starts from the `hello` example. |
| [Scripting and modding](scripting.md) | The Luau host: sandbox, hot reload, step budget. Growing Guns' mod API as the worked example. |
| [Audio](audio.md) | Engine sound synthesis, effects playback, and the procedural synth with its surround mixer. |
| [Live control over MCP](live-control.md) | Change a running game with Claude, ChatGPT or an in-game assistant: setup, tools, security. |
| [Networking](networking.md) | Host and clients over iroh, and Growing Guns' host-authoritative play. |
| [Physics](physics.md) | `kern::collide` for characters and levels, the Rapier-based `physics` crate. |
| [Live sky: time of day and weather](live-sky-controls.md) | `render::sky`: presets, the background bake, Sideways' pause menu and Growing Guns' `--sky` and `sky` command. |
| [Rendering overview](rendering.md) | Clusters, visibility buffer, lighting, content systems, and where performance stands. |
| [Quarried stone surfaces](stone-surfaces.md) | Shared stone meshes, physical normal relief, footprint filtering and the Growing Guns prototype's remaining visual issues. |

Reference that lives next to the code:

- [Growing Guns mod API](../../games/growing-guns/mods/API.md), generated
  from the Rust side.
- [Frame budget and repeatable scenarios](../tools/frame-budget.md) and the
  [diagnostics tools](../tools/README.md).
- [Engine README](../README.md) and the [repository README](../../README.md).

## Engineering notes

Dated notes written during development, kept as they were written: what
was tried, what was measured, and what was accepted or rejected. They are
evidence, not tutorials. Numbers in them belong to the build and scene
they name. Iteration numbers (r34, r52...) follow the order of work.

Status and plans:

- [Handoff](../HANDOFF.md): the running status, newest first.
- [Forest rendering roadmap](forest-rendering-roadmap.md) (in Dutch).
- [A second game on Warp: findings](second-game-findings.md).
- [Iteration speed measurements, 24 October 2026](iteration-speed-2026-10-24.md).
- [Engine efficiency changes, 28 October 2026](engine-efficiency-20261028.md): seven runtime fixes, CPU allocation measurements and parity evidence.
- [Native workspace separation](repository-layout.md).
- [GPU runner](../GPU_RUNNER.md) and [Windows validation, 2026-10-23](../WINDOWS_VALIDATION.md).

Meadow and grass:

- [Procedural meadow field, r34](meadow-field-r34.md)
- [Meadow appearance, r35](meadow-look-r35.md)
- [Dense meadow, r36](meadow-density-r36.md)
- [Meadow geometry budget, r37](meadow-budget-r37.md)
- [Meadow distance continuity, r39](meadow-distance-r39.md)
- [Grass distribution and a faint night sky, r40](meadow-night-r40.md)
- [Meadow budget, r44](meadow-budget-r44.md)
- [Meadow world-page cache, r45](meadow-group-cache-r45.md)
- [Meadow work elimination and optical bricks, r47](meadow-work-r47.md)
- [World-fixed vegetation volume field](meadow-volume-field.md)

Trees and vegetation:

- [Unified vegetation: shared primitives and branch poses](unified-vegetation.md)
- [Vegetation work by distance, r31](vegetation-distance-budget.md)
- [Vegetation source LOD and volume work, r32](vegetation-optimization-r32.md)
- [Forest fullness and current-frame culling, r38](forest-occlusion-r38.md)
- [World foliage replacement, r50](world-foliage-r50.md)
- [Distance-based tree wind freeze and static sun cache, r52](wind-freeze-sun-cache-r52.md)
- [Distant mountains and forest](distant-forest.md)
- [Distant tree sunlight](distant-tree-lighting.md)

Traversal and work avoidance:

- [Instance hierarchy, r45](instance-hierarchy-r45.md)
- [Rendering less work, r46](render-work-avoidance-r46.md)
- [Cluster traversal, r46](cluster-traversal-r46.md)
- [Real traversal avoidance, r48](render-traversal-r48.md)
- [Compact colour-view instance selection](instance-broadphase.md)
- [Controlled colour-geometry diagnostics, 24 October 2026](render-categories-2026-10-24.md)

Shadows and lighting:

- [Matched grass optics and receiver-driven shadow work, r49](lod-shadow-demand-r49.md)
- [Current-pose directional shadow demand](receiver-shadow-demand-plan.md)
- [Static/dynamic sun cache, r45](static-sun-cache-r45.md)
- [Low-sun shadow diagnostics](sun-shadow-diagnostics.md)
- [World visibility and cached lighting](world-lighting.md)
- [Shared world-space diffuse radiance cache](radiance-cache.md)
- [Next live diffuse-bounce step (design only)](live-terrain-bounce-plan.md)
- [Saved vegetation, vehicle illumination and bloom, r51](local-lights-and-vegetation-r51.md)

Sky, fog and terrain:

- [Procedural atmosphere foundation](../ATMOSPHERE.md)
- [Live sky controls](live-sky-controls.md)
- [Camera terrain occlusion of solar haze](camera-sun-fog.md)
- [Finite-distance aerosol layer](finite-distance-fog.md)
- [Volumetric light from local lights](volumetric-light.md)
- [Terrain normals follow the rendered surface](terrain-surface-normals.md)
- [Shared terrain visibility foundation](terrain-world-visibility.md)
- [Repetitive roadside wedges, r52](roadside-verge-r52.md)
- [Disappearing roadside shadow, r53](roadside-shadow-r53.md)

Vehicles and effects:

- [Car headlights](car-headlights.md)
- [Rear-lens emission](car-rear-lamps.md)
- [Exhaust gas, backfire and heat](exhaust-effects.md)
- [Wheel particles and tyre marks](wheel-effects.md)
- [Wheel dust and debris](wheel-particles.md)
- [Persistent tyre material marks](persistent-tyre-marks.md)
- [Roadside crowds prototype](roadside-crowds.md)

Tools:

- [Live render debug panel](debug-ui.md)
- [Processed texture cache](processed-texture-cache.md)
