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
