# Development log

Notes written while Warp was being built: what was tried, what was measured,
and what was accepted or rejected. They are evidence, not documentation of
how things work now. Numbers in them belong to the build, machine and scene
they name, and a later note or the code may have overtaken them. Iteration
numbers (r34, r52 and so on) follow the order of work.

For how things work now, read the [guides](../README.md) and
[status](../status.md). Switches named in these notes are listed, with their
current defaults, in [renderer switches](../renderer-switches.md).

## Writing a log entry

- A new experiment, measurement or investigation gets its own file here,
  named `YYYY-MM-DD-topic.md`, with the date, commit, machine, scene and
  result. Add it to the list below.
- Do not edit an old entry to make it current. When it is overtaken, change
  the guide or [status](../status.md) instead.
- When a change alters how something works, update the guide for that part
  in the same commit. The log is not a substitute for that.
- [Port Meridian lawns, cloud shadows and four-player frame times](2026-11-22-warp-theft-auto-lawns-weather.md)
- [Port Meridian interiors, residents and four-player frame times](2026-11-22-warp-theft-auto-interiors.md)
- [Warp Theft Auto in four views at 1080p and 4K](2026-11-22-warp-theft-auto.md)
- [Warp Human face texture memory](2026-11-22-warp-human-skin-memory.md)
- [Terrain Fly patch selection, forest streaming and sun shadows](2026-11-21-terrain-fly-performance.md)
- [Voxel rasterization and brick culling](2026-11-21-voxel-render-cull.md)
- [Swiss forest shadow replay and depth-culling measurements, 18 November](2026-11-18-forest-shadow-replay.md)
- [Vegetation cost, tiers and rejected candidates, 18 November](2026-11-18-vegetation-cost-and-tiers.md)
- [Vegetation pop-in: handoff, 18 November](2026-11-18-pop-in-handoff.md)
- [Forest pop-in measurement and handoff, 18 November](2026-11-18-forest-pop-in.md)
- [handoff.md](handoff.md) is the running log of 23 October to 5 November
  2026, newest first.

- [Stadium broadcast camera costs](2026-11-11-stadium-broadcast.md)
- [SpaceRacer rule calls: where a sim tick went](2026-11-11-spaceracer-rule-calls.md)
- [Sideways GPU cost: LOD error, culling switches, car shading, shadow distance LOD](2026-11-11-sideways-gpu-cost.md)
- [Hair rendering in games and a scalp-rooted Warp experiment](2026-11-11-warp-humans-hair-research.md)
- [The side-parted bob in three views](2026-11-11-warp-humans-bob-volume.md)
- [Close hair guides and geometric fibres](2026-11-11-warp-humans-hair-guides.md)
- [Keeping seeded facial proportions within adult reference heads](2026-11-11-warp-humans-proportions.md)
- [Close faces, gaze and eyelids](2026-11-10-warp-humans-close-faces.md)
- [Skin tone and seeded hair colour](2026-11-10-warp-humans-hair.md)
- [Fitted player stance, face and sparse body asset](2026-11-10-warp-humans-fit.md)
- [Luau call costs and Rust rebuild times](2026-11-10-luau-call-cost.md)
- [Growing Guns reload grip and GameNight colours](2026-11-09-warp-humans-reload.md)
- [Distance-matched player locomotion and jump contacts](2026-11-09-warp-humans-player-locomotion.md)
- [Spellstorm spell effects, first design pass](2026-11-09-spellstorm-spell-effects.md)
- [Shared wardrobe, body proportions and planted motion](2026-11-09-warp-humans-wardrobe.md)
- [Seeded match players and spectator debris](2026-11-09-warp-humans-players-and-gibs.md)
- [Blood stains under stadium lights](2026-11-09-growing-guns-blood-reflections.md)
- [Turning away from a blast](2026-11-09-warp-humans-blast-turn.md)
- [Startled spectator gestures](2026-11-09-warp-humans-startle.md)
- [Planted steps and crouches on the terraces](2026-11-09-warp-humans-footwork.md)
- [Stadium voices and spectator gestures](2026-11-09-stadium-voices-and-gestures.md)
- [Warp Humans in stadium fog](2026-11-09-warp-humans-fog.md)
- [Growing Guns blood spray and surface contacts](2026-11-09-growing-guns-blood-spray.md)
- [Repeated crowd shots and blast reactions](2026-11-09-growing-guns-crowd-reactions.md)
- [Packed seating in the Growing Guns match](2026-11-09-growing-guns-crowd-density.md)
- [Growing Guns world scale and standing eye height](2026-11-09-growing-guns-scale.md)
- [Rigged people and Growing Guns frame times](2026-11-09-warp-humans-rig.md)
- [Seeded stadium crowd from stands to face](2026-11-08-warp-humans.md)

## Status and plans

- [Vegetation takeover handoff, 6 November 2026](2026-11-06-vegetation-handoff.md)

- [Handoff log, 23 October to 5 November 2026](handoff.md)
- [Forest rendering roadmap](forest-rendering-roadmap.md) (in Dutch; overtaken by
  the [vegetation checklist](../vegetation-architecture.md))
- [Foliage as assemblies: plan](foliage-assemblies-plan.md)
- [GPU forest integration contract](foliage-no-cpu-pool-integration.md)
- [Vegetation next-agent hand-off, 29 October](vegetation-next-agent.md)
- [Four-view render budget plan, r55](four-view-budget-plan-r55.md)
- [A second game on Warp: findings](second-game-findings.md)
- [Native workspace separation, r29](repository-layout.md)
- [Windows validation, 23 October 2026](windows-validation-2026-10-23.md)

## Performance and tooling

- [Parenting in four games, 7 November](2026-11-07-parenting.md)
- [One sun clipmap: cached levels replace the shared and far maps, 8 November](2026-11-08-sun-clipmap.md)
- [Screen-space refraction, 8 November](2026-11-08-refraction.md)
- [Material maps, weathering, BC7 and texture streaming, 8 November](2026-11-08-materials-and-texture-streaming.md)

- [Iteration speed, 24 October](iteration-speed-2026-10-24.md)
- [Colour-geometry category costs, 24 October](render-categories-2026-10-24.md)
- [Engine efficiency changes, 28 October](engine-efficiency-20261028.md)
- [DLSS resolution sweep, r55](resolution-sweep-r55.md)
- [Visible foliage-page hitches, r61](frame-hitches-r61.md)
- [SpaceRacer native 4K, 1 November](spaceracer-native4k-2026-11-01.md)
- [Quest 2 cost investigation, 31 October](quest-profile-2026-10-31.md)
- [Quest lightweight graphics, 1 November](quest-optimization-2026-11-01.md)
- [terrain_fly in a browser (wasm, WebGPU): feasibility, 5 November](2026-11-05-browser-feasibility.md)
- [Shading without local lights, 5 November](2026-11-05-shade-without-local-lights.md)
- [Land shading costs, 6 November](2026-11-06-land-shading-costs.md)
- [Clouds, multiple scattering and aerial perspective, 8 November](2026-11-08-clouds-and-aerial-perspective.md)

## Traversal and work avoidance

- [Instance hierarchy, r45](instance-hierarchy-r45.md)
- [Rendering less work, r46](render-work-avoidance-r46.md)
- [Cluster traversal, r46](cluster-traversal-r46.md)
- [Real traversal avoidance, r48](render-traversal-r48.md)
- [Compact colour-view instance selection](instance-broadphase.md)

## Trees and the GPU forest

- [Vegetation work by distance, r31](vegetation-distance-budget.md)
- [Vegetation source LOD and volume work, r32](vegetation-optimization-r32.md)
- [Forest fullness and current-frame culling, r38](forest-occlusion-r38.md)
- [World foliage replacement, r50](world-foliage-r50.md)
- [Foliage PR integration and trials, 26 October](pr-foliage-trials-2026-10-26.md)
- [Unified vegetation: shared primitives and branch poses](unified-vegetation.md)
- [Species recipes from shared primitives](vegetation-species.md)
- [Parallel brick visibility, 29 October](vegetation-brick-visibility.md)
- [Rasterized foliage voxels plan, 30 October](voxel-raster-plan.md)
- [Terrain fly voxel culling, 3 November](voxel-brick-culling-2026-11-03.md)
- [Retained terrain forest resources, 3 November](terrain-forest-streaming-2026-11-03.md)
- [GPU foliage selection core, 3 November](foliage-gpu-selection-2026-11-03.md)
- [GPU forest runtime, 4 November](foliage-runtime-2026-11-04.md)
- [GPU subtree work queues, 4 November](foliage-wavefront-2026-11-04.md)
- [Stable streamed population, 4 November](foliage-population-2026-11-04.md)
- [History-first source recovery, 4 November](foliage-history-2026-11-04.md)
- [Shared articulated poses, 4 November](foliage-shared-poses-2026-11-04.md)
- [Render suite and first iterations, 5 November](2026-11-05-render-suite.md)
- [Asset demand and publication, 4 November](foliage-asset-publication-2026-11-04.md)
- [Relocatable voxel pages, 4 November](foliage-page-layout-2026-11-04.md)
- [Bounded voxel asset cache, 4 November](foliage-cache-2026-11-04.md)
- [Source paging, 4 November](foliage-source-pages-2026-11-04.md)
- [Resident intermediate cuts, 4 November](foliage-resident-cuts-2026-11-04.md)
- [Shadow tile damage and sparse clear, 4 November](foliage-shadow-damage-2026-11-04.md)
- [Mixed species and grass aggregates, 4 November](foliage-mixed-species-2026-11-04.md)

## Meadow and grass

- [Procedural meadow field, r34](meadow-field-r34.md)
- [Meadow appearance, r35](meadow-look-r35.md)
- [Dense meadow, r36](meadow-density-r36.md)
- [Meadow geometry budget, r37](meadow-budget-r37.md)
- [Meadow distance continuity, r39](meadow-distance-r39.md)
- [Grass distribution and a faint night sky, r40](meadow-night-r40.md)
- [Meadow budget, r44](meadow-budget-r44.md)
- [Meadow world-page cache, r45](meadow-group-cache-r45.md)
- [Meadow work elimination and optical bricks, r47](meadow-work-r47.md)
- [World-fixed vegetation volume field (plan)](meadow-volume-field.md)
- [Meadow handoff band, r56](meadow-band-r56.md)
- [Earlier, reduced-resolution aggregate grass, r57](meadow-half-resolution-r57.md)
- [Distant grass sampling cost, r58](meadow-sampling-r58.md)
- [Grass contact AO, r59](meadow-contact-ao-r59.md)
- [Rounded blade shading, r62](meadow-rounded-shading-r62.md)
- [Curved grass and travelling wind, r63](meadow-windswept-r63.md)
- [Distant grass shading and edge contact, r64](meadow-edge-grain-r64.md)
- [Windswept grass integration, 28 October](grass-integration-20261028.md)

## Shadows and lighting

- [Live bounce of moving lights (reflective shadow maps), 12 November](2026-11-12-live-bounce-rsm.md)
- [Reflections: light faces out of the mips, 12 November](2026-11-12-reflections-light-faces.md)
- [Live reflection probes, 12 November](2026-11-12-reflection-probes.md)
- [Thin bounce voxels, whole-probe captures, shade's texture count, 12 November](2026-11-12-thin-voxels-probe-frames.md)
- [Froxel fog, 12 November](2026-11-12-froxel-fog.md)
- [Grain in the traced reflections of a moving surface (open), 12 November](2026-11-12-moving-reflector-grain.md)
- [The room bake on the GPU, 13 November](2026-11-13-gpu-room-bake.md)
- [Room bake: bias of the smoothing, noise of small openings, kinks, 13 November](2026-11-13-probe-noise-bias.md)
- [Five storage textures in one stage: terrain_fly would not start, 13 November](2026-11-13-storage-texture-limit.md)
- [Froxel fog: dark edges, a flickering horizon, flicker under a lamp, rooms, 13 November](2026-11-13-fog-edges-flicker.md)
- [What the probes' gradients cost in shade, 14 November](2026-11-14-gradient-cost.md)
- [The grain on the swinging pendant's shade, checked again, 14 November](2026-11-14-pendant-shade-grain.md)
- [The fog's cone edge as a staircase near the camera, 14 November](2026-11-14-fog-cone-staircase.md)
- [The green grain on the pendant shade's inside: SSR noise (open), 14 November](2026-11-14-shade-grain-ssr.md)
- [Traced reflections, stage 1, 8 November](2026-11-08-traced-reflections.md)
- [Low-sun shadow diagnostics, 24 October](sun-shadow-diagnostics.md)
- [World visibility and cached lighting, 24 October](world-lighting.md)
- [Next live diffuse-bounce step (design only)](live-terrain-bounce-plan.md)
- [Static/dynamic sun cache, r45](static-sun-cache-r45.md)
- [Current-pose directional shadow demand](receiver-shadow-demand-plan.md)
- [Matched grass optics and receiver-driven shadows, r49](lod-shadow-demand-r49.md)
- [Saved vegetation, vehicle lights and bloom, r51](local-lights-and-vegetation-r51.md)
- [Wind freeze and static sun cache, r52](wind-freeze-sun-cache-r52.md)
- [Distant tree sunlight](distant-tree-lighting.md)
- [Stadium diffuse sky and sun transport](stadium-sky-occlusion.md)

## Terrain and roadside

- [Repetitive roadside wedges, r52](roadside-verge-r52.md)
- [Disappearing roadside shadow, r53](roadside-shadow-r53.md)

## Effects and content

- [Warp Humans posture in Luau games, 18 November](2026-11-18-warp-humans-posture.md)
- [Warp Humans skin and eyes from the seed, 18 November](2026-11-18-warp-humans-seed-skin.md)
- [Warp Humans hair grown from the seed, 18 November](2026-11-18-warp-humans-grown-hair.md)
- [Warp Humans sun shadow on skin, 18 November](2026-11-18-warp-humans-skin-shadow.md)
- [Warp Humans haircuts instead of lengths, 18 November](2026-11-18-warp-humans-haircuts.md)
- [Warp Humans middle age and old age, 19 November](2026-11-19-warp-humans-ageing.md)
- [Warp Humans shine through an afro, 20 November](2026-11-20-warp-humans-scalp-shine.md)
- [GPU particles first pass, 8 November](2026-11-08-gpu-particles.md)
- [Baked smoke scalability baseline, 31 October](baked-smoke-scaling.md)
- [Growing Guns crowd silhouettes, 29 October](growing-guns-crowd.md)
