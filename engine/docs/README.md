# Warp documentation

Warp is a Rust game engine designed for fast iteration with AI. Everything
is code and text, visual [editor tools](editor-tools.md) run inside games, every run replays bit for bit,
and a running game can be inspected and changed by an AI over MCP. It is
made for couch games with up to four players on one screen. Its focus so
far is procedural outdoor worlds, with headless rendering, scripted
playtests and hot-reloaded Luau.

Games on it: [Sideways](../../games/sideways/README.md), a gravel rally
game, and [Growing Guns](../../games/growing-guns/README.md), an arena
shooter. [SpaceRacer](../../games/spaceracer/README.md) has a playable native
development port with Luau rules. Spellstorm, a co-op wizard roguelite, and
Splitscreen Club's Game Room lobby are being ported. The `warp` executable runs
games made only of Luau scripts and sound patches, such as
[Hello Warp](../../games/hello-warp/README.md), with no Rust.

Start with the overview, then build and run something. [Status](status.md)
says what works now and what is still open.

These pages describe how Warp works now. Dated experiments, measurements
and session notes are in the [development log](log/README.md).

## Guides

| Guide | What it covers |
| --- | --- |
| [Playtester F12 captures](warp-cli.md#sideways-feedback) | Replay Sideways driver input, inspect a pixel by coordinate, or compare physics, clips and GPU timings across revisions. [Growing Guns input and live-edit history](../../games/growing-guns/README.md#replay-a-moment-while-editing-graphics). |
| [Warp Humans](warp-humans.md) | Run the two-view seeded stadium crowd and compare distant representations. |
| [Procedural character contacts](animation.md) | Layer a reachable prop socket over an animated arm and render a repeatable grip preview. |
| [What Warp is](overview.md) | Design goals: deterministic sim, fast iteration, headless runs, physical units, live tweaking by an AI as a direction. |
| [Status](status.md) | What is the default, what is opt-in and what is open, per area. |
| [Workspace and crates](workspace.md) | The repository layout, the engine crates, the game packages and the build profiles. |
| [Building and running](building-and-running.md) | Checks and tests, running the games, software rendering, scripted playtests, golden images, environment variables. |
| [Running in a browser](browser.md) | terrain_fly built for wasm32 and WebGPU: building it, what a browser needs, how that build differs, what is open. |
| [Parenting and attachment points](parenting.md) | Luau entities, native model sockets and posed instance groups. |
| [Making a game](making-a-game.md) | The `app::Game` trait, the loop, building a scene, costs and gotchas. Starts from the `hello` example. |
| [Games in Luau: the warp executable](warp-cli.md) | `warp new`, `run`, `script`, `serve`, `check`: a game of Luau scripts, sound patches and glTF assets with hot reload and live MCP control. The Luau game API. |
| [Luau or Rust](luau-or-rust.md) | Where game code goes: rules and content in Luau, loops over many things in Rust, per-pixel and per-instance work on the GPU. What a crossing costs, the patterns to copy, moving code across. |
| [Scripting and modding](scripting.md) | The Luau host: sandbox, hot reload, step budget. Growing Guns' mod API as the worked example. |
| [Audio](audio.md) | Engine sound synthesis, effects playback, and the procedural synth with its surround mixer. |
| [Live control over MCP](live-control.md) | Change a running game with Claude, ChatGPT or an in-game assistant: setup, tools, security. |
| [Networking](networking.md) | Host and clients over iroh, and Growing Guns' host-authoritative play. |
| [Physics](physics.md) | `kern::collide` for characters and levels, the Rapier-based `physics` crate. |
| [Rendering overview](rendering.md) | Clusters, occlusion, visibility buffer, lighting, content systems, and where performance stands. |
| [Materials and textures](materials.md) | Material maps, wet/snow/moss weathering, a game's own surfaces, BC7 compression and texture streaming. |
| [Renderer switches](renderer-switches.md) | Every `KERN_*` variable the renderer reads, its default, and which are experiments. |
| [Procedural terrain](terrain.md) | The height function and its GPU copy, LOD patches, water, constraints, forest field and placement. |
| [Terrain regions](terrain-regions.md) | Several kinds of land in one world. |
| [Zones and footprints](terrain-zones.md) | What each place is for, flat plots, buildings in `terrain_fly --town`. |
| [World overrides](world-overrides.md) | Artist edits on top of rule placement, keyed on stable object ids. |
| [Vegetation checklist](vegetation-architecture.md) | The GPU forest and grass route: architecture contract, gates and current measurements (a working checklist). |
| [Atmosphere](atmosphere.md) | The procedural sky bake (Rayleigh and Mie, multiple scattering, ozone, sky lighting), aerial perspective and volumetric clouds. |
| [Live sky: time of day and weather](live-sky-controls.md) | `render::sky`: presets, the background bake, Sideways' pause menu and Growing Guns' `--sky` and `sky` command. |
| [Exposure and HDR output](hdr-and-exposure.md) | Automatic exposure and HDR output on HDR screens: when to use them, how to turn them on, authoring for HDR. |
| [Light in rooms](interior-light.md) | Baking sky and sun light for buildings so rooms are darker than outside: multi-bounce probes from the scene's triangles, no leaks through walls, dimmed reflections. |
| [Buildings](architecture.md) | Homes inside and out from glTF (AtomicBuildings, IFC via IfcConvert): the `architecture` crate, room bake grid, room and outside shots. |
| [Skin and hair](skin-and-hair.md) | Skin and hair lighting for characters: what the SKIN and HAIR materials do, masking a character, the Growing Guns review. |
| [Temporal upscaling](temporal-upscaling.md) | The portable upscaler and DLSS modes, motion vectors for skinning and wind. |
| [Motion blur](motion-blur.md) | Opt-in per-view motion vectors, fixed shutter, split-screen isolation and limitations. |
| [Depth of field](depth-of-field.md) | Opt-in thin-lens blur per view: f-number, fixed or GPU autofocus, cost and limits. |
| [Volumetric light](volumetric-light.md) | Opt-in in-scattering from local lights. |
| [Water optics](water.md) | Opt-in refraction and screen-space reflections on terrain water. |
| [Live render debug panel](debug-ui.md) | F10: render and light switches, LOD, GPU pass times. |
| [Stats overlay](stats-overlay.md) | F3: frame cost while playing, a nested GPU tree, CPU spans, memory, game lines. |
| [Render suite](render-suite.md) | One fixed world from fixed cameras: per-pass GPU times, counters, memory and pictures, with diffs between runs. |
| [Native profiling](native-profiling.md) | No-readback headless CPU/GPU samples and per-frame reports. |
| [Native Quest VR](quest-native.md) | Android/OpenXR with the Warp renderer and the full Growing Guns app; build, controls and validation limits. |
| [Standalone Quest VR demo](quest-vr.md) | WebXR shell, local Rust/Wasm Growing Guns demo, Touch controls. |
| [GPU runner](gpu-runner.md) | Running GPU jobs with Claude Code on a Windows machine with a real GPU. |

## Reference

Narrower notes on single features.

Lighting and terrain:

- [Shared terrain visibility](terrain-world-visibility.md)
- [Dynamic GI and reflections](dynamic-gi.md): traced reflections, open stages
- [Shared world-space diffuse radiance cache](radiance-cache.md)
- [Camera terrain occlusion of solar haze](camera-sun-fog.md)
- [Finite-distance aerosol layer](finite-distance-fog.md)
- [Terrain normals follow the rendered surface](terrain-surface-normals.md)
- [Distant mountains and forest](distant-forest.md)
- [Processed texture cache](processed-texture-cache.md)

Vehicles and effects:

- [Car headlights](car-headlights.md)
- [Rear-lens emission](car-rear-lamps.md)
- [Exhaust gas, backfire and heat](exhaust-effects.md)
- [Wheel particles and tyre marks](wheel-effects.md)
- [Wheel dust and debris](wheel-particles.md)
- [Persistent tyre material marks](persistent-tyre-marks.md)
- [Baked explosion smoke](baked-explosion-smoke.md)
- [Adaptive explosion smoke](adaptive-explosion-smoke.md)
- [GPU particles](gpu-particles.md)
- [Roadside crowds prototype](roadside-crowds.md)

Materials:

- [Quarried stone surfaces](stone-surfaces.md)
- [Stadium masonry](stadium-masonry.md)

Next to the code:

- [Growing Guns mod API](../../games/growing-guns/mods/API.md), generated
  from the Rust side.
- [Frame budget and repeatable scenarios](../tools/frame-budget.md) and the
  [diagnostics tools](../tools/README.md).
- [DLSS bridge](../crates/render/native/DLSS.md).
- [Engine README](../README.md) and the [repository README](../../README.md).
