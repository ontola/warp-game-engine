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
- [handoff.md](handoff.md) is the running log of 23 October to 5 November
  2026, newest first.

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

- [GPU particles first pass, 8 November](2026-11-08-gpu-particles.md)
- [Baked smoke scalability baseline, 31 October](baked-smoke-scaling.md)
- [Growing Guns crowd silhouettes, 29 October](growing-guns-crowd.md)
