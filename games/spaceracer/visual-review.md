# Exhaust and aircraft corrections - 31 October 2026

Baseline: 7488c0b. Source `ship.gd`, `racer_design.gd`, plasma/glow/flare/
boost-trail shaders inform the native implementation; retained assets unchanged.
No retired runtime or export tools were run.

- Replaced intersecting static white plume sheets with per-view procedural
  plasma: narrow hot core, coloured transparent envelope and moving shock cells.
- Restored nozzle glows, source-occluded horizontal flares, actual background
  heat refraction, orange reverse jets and short soft exhaust wisps. Wisps move
  backwards relative to the craft faster than its forward speed.
- Throttle drives the nozzle even during countdown; acceleration/boost extend
  the plume. Coloured local lights share that same state. EMP, crash and
  recovery suppress them immediately; releasing throttle fades smoothly.
- Boost adds irregular discharges that change on each advancing presentation
  frame, and 0.38-second world-space phosphor trails. Pause freezes effects;
  crash/recovery/teleport clears trail history. No per-frame mesh generation.
- Restored source YXZ component replacement for wings, rudders and brakes;
  preserves authored sweep/cant and clamps wing travel. Weapon mounts use the
  source universal socket (0, 1.15, -1.8), instead of floating higher/forward.
- `rules/jets.luau` holds effect tuning and anchors, read when generating a race.
  GPU shading, bounded geometry/history and per-frame interpolation stay native.

## Evidence

26 focused sim/app checks pass. `scenarios/jet-effects.json` captures 24 advancing
boost frames, countdown, throttle, brake and pause (601 ticks). It uses a virtual
controller, not physical hardware. The existing 1,207-tick controller/pause/EMP
scenario also passes. The initial test script incorrectly sent analog triggers
to a keyboard seat; it was corrected to a virtual pad before the final run.

Artifacts in `build/spaceracer-jets/`: `city.png`, `forest.png`, `four.png`,
`effects/boost-18.png`, `effects/boost-motion.webp`, `effects/countdown-throttle.png`,
`effects/brake.png` and `controller-flow/`. City/day/four-view images were inspected.
The moving sequence advances two 120 Hz ticks per frame. Pause effect buffers
are identical in the unit check; complete pause screenshots differ by about
0.47/255 mean RGB as temporal reconstruction settles, so pixel identity is not
claimed. Hardware controllers and production GameNight were not tested.

Four-view city, seed 31, 750 warmup ticks, 1920x1080, 120 moving frames,
RTX 5070 Ti / Vulkan / NVIDIA 617.14: **81.11 headless readback frames/s**.
Smoothed reported sprite and exhaust passes were 0.050 and 0.047 ms respectively.
These counters are not full-frame timings and must not be summed across views.
Background load was not isolated; no steady 120 fps or improvement over the
prior run is claimed. Report: `four-bench.json`.

This improves the exhaust and aircraft details; full weapon meshes, visor,
wear/debris/respawn presentation and the listed missing stage features remain.
The source formulas are adapted to Warp, not a claim of identical rendering.

---

# Stage restoration review - 31 October 2026

Baseline: native commit 08a9888, which still replaced original stages with
sparse generic objects. This pass reads the retained MIT source recipes and
assets; no retired runtime or export pipeline is used. Retained asset bytes
are unchanged. New scenery collision/terrain is intentional; committed
handling, flight and track fixture outputs remain unchanged.

## Restored content

- City: original eight building families, six roof families, 120 m block grid,
  mixed lots, tall opening landmarks, aerials, mounted source artwork, office
  texture/shader and separate ground/air traffic. Seed 31 has 699 moving cars.
- Road: bounded analytic reflections of actual nearby screens and facade
  boxes. A 128 m spatial grid selects six screens/twelve boxes, sampled using
  the same artwork/material mapping. This is not complete scene ray tracing.
- Forest: original pine/birch assets at source age ranges (up to 650 m), ferns,
  noise landscape, meadow/stone/shore materials and shoreline-aware water.
- Cell: continuous double helices, knotted proteins, mitochondrial folds,
  layered reticulum, nucleus/pores, 31 animated walkers in seed 31 and the
  source Voronoi enclosing membrane. Fixed overlapping backfaces.
- Desert/ocean: real shared heightfields, source mesa/reef and canyon equations,
  sandstone strata, hoodoos, cacti/arches or kelp/lanterns/fans/sponges.
  Ocean is submerged, with fish schools, jellyfish and manta rays.
- City and cell use an authored directional ambient probe volume independent
  of visible sky brightness. Vehicles and asphalt stay legible at night;
  this is artistic ambient light, not a baked GI claim. Ocean ambient is raised.
- All five retained ship designs display; stage animation uses interpolated
  race time and freezes with pause.

Composition is in `rules/stages.luau`; geometry, terrain sampling, mesh sharing,
clearance and per-frame transforms remain native. The stage file is re-read
when generating a race. Source recipes are adapted to the native track,
spatial queries and rendering, not bit-identical scene reproduction.

## Reproduction and evidence

`tools/build-native.ps1` builds the portable executable. Captures and reports
are in `build/spaceracer-faithful/` (ignored artifacts):

- City: seed 31, 750 ticks, 1600x900, `city-final.png`.
- Forest: seed 35, 700 ticks, 1600x900, `forest-final.png`.
- Cell/desert: seed 31, 750 ticks, 1600x900, `*-final.png`.
- Ocean: seed 35, 1600 ticks, 1600x900, `ocean-final.png`.
- Controller/pause/EMP flow: `scenarios/native-race.json`, `flow/`.
- Four-view city: 1920x1080, 120 moving headless frames, seed 31/tick 750.

23 focused simulation/app tests pass, including stage composition and road
clearance in five worlds, terrain triangle interpolation and ridge sweeps.
Shots warm eight stationary frames; screenshots alone do not validate moving
image quality. Real controllers and the production host are not re-tested.

## Performance and remaining differences

RTX 5070 Ti / Vulkan / NVIDIA 617.14, optimized dev build. Four-view city
measured **158.74 readback frames/s** in the final run over 120 frames at
1920x1080; this is not
interactive FPS or a 4K/120 claim. See `city-four-final-bench.json`. Smoothed GPU
samples include shade 0.689 ms, raster 0.226 ms, cull 0.260 ms; they are not
full-frame percentiles and must not be summed across the split-screen views.
Background CPU load was not isolated; an earlier run measured 78.25 readback
frames/s, so throughput is sensitive to the run conditions. The restored city has 50,152 instances;
it is materially heavier than the sparse prior port.

Full parity remains incomplete: source vista protections, desert phase/slot/
basin specialization and sandworm, giant octopuses, bubbles/snow/light shafts,
full water/foliage/creature animation, visor and weapon/damage presentation.
Tree/organic collisions are bounds proxies. These omissions are documented,
not replaced by a claim that the screenshots now match the original exactly.

---

# Native visual review — 31 October 2026

Baseline: native commit b0abd49. No retired runtime or export workflow used.
All retained source assets are unchanged. Simulation geometry, collision bounds
and random-number sequence are preserved; Luau city/cell lighting values changed.

## Reproduction

Use the packaged executable from `tools/build-native.ps1` with:

- City: `--demo --seed 31 --ticks 750 --players 2 --shot build/city.png`.
- Four views: `--demo --seed 31 --ticks 960 --players 4 --width 1920 --height 1080 --shot build/four.png`.
- Other worlds: `--demo --seed 35 --ticks 700 --biome forest --shot build/forest.png`
  (also cell, desert and ocean).

Local before/after captures are in `build/spaceracer-visual-fix/`.
The before-city capture used the previous executable immediately before rebuilding.
The before-forest/four captures are retained outputs of b0abd49.
Screenshots warm eight stationary frames; they do not establish moving-image quality.

## Corrections

- Six corporate posters were incorrectly treated as an eight-tile atlas.
  Correct tile coordinates, outward orientation and poster proportions.
- Windows no longer stretch to fill the height of each tower. Warm/cool-white
  office groups have metre-sized floors, mullions and occupancy variation.
- Hull surfaces restore authored marking styles without floating decal meshes;
  retained emissive marker strips are rendered again.
- Road material has filtered grain, panel seams, lane/edge markings and varied
  roughness. Fascias make elevated ribbons solid at the edge.
- Local track fixtures and recalibrated jet lighting produce bounded highlights.
  Light selection is capped to four track fixtures per player and 32 total sources.
- Exhaust replaces overlapping glowing motes with a soft tapered mesh. Texture
  padding and tapered geometry prevent repeat-sampler stars at the far end.
- Daytime atmosphere, less haze, thin-foliage material flags, stone silhouettes
  and quieter water normals replace several flat placeholder materials.
- Pickups have open luminous frames instead of solid glowing cubes.

## Measurements and limits

RTX 5070 Ti, Vulkan, NVIDIA 617.14, optimized development build. Seed 31,
four views at 1920x1080, 120 headless frames after 750 simulation ticks.
The final revision measured 195.4 readback frames/s, recorded in `build/spaceracer-visual-fix/bench-final.json`. This includes CPU
submission/readback and is not an interactive FPS claim. GPU pass samples are
smoothed engine counters, not full-frame latency percentiles. Background CPU
load was not isolated. No original-engine performance comparison is claimed.

20 focused simulation/native-app tests and the 1,207-tick controller/pause/EMP
scenario pass. Five worlds and one/two/four-view captures were rendered.
Real controllers and production GameNight were not re-tested in this visual pass.

Full scene reflections (including billboard/window images), city traffic,
complete biome dressing, visor effects and damage/respawn presentation remain
incomplete. The build is still a development port, not full visual parity.
