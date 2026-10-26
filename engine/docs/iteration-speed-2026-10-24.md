# Iteration speed measurements — 24 October 2026

Keep measurements on each developer launch and bounded scenario. The aim is
to shorten edit → build → first scene → useful evidence while retaining honest
frame-budget measurements. Do not compare Godot; Warp is the active engine.

## Implemented

- Four-slot asynchronous GPU timestamp readback, joined to CPU records by frame
  ID. Loading, warmup, paused and diagnostic captures are separately labelled.
- TOML scenarios define seed, views, output, fixed cameras, exact initial physics
  tick, contiguous sections, sun presets and selected captures. A scene/renderer
  is reused across sections. This is not a reconstruction of historical controller
  inputs or wheel trails. Examples and command syntax: [frame-budget](../tools/frame-budget.md).
- Continuous startup timing includes pre-window scene work, renderer construction,
  DLSS preparation and first scene submission. `play-dev-windows.ps1` saves it.
- SHA-256 keyed, checksummed, atomic processed-texture cache. Invalid entries
  rebuild; source assets are never changed. [Protocol](processed-texture-cache.md).
- Scenario previews are JPEG1600/85 by default. GPU downsampling reduces readback;
  the full HUD is composed before scaling. Native PNG is explicit; F12 unchanged.
- `iteration-report.py` reports each workflow/configuration separately. It does
  not average different view counts, capture policies, resolutions or executables.

## Measured on this workstation

Evidence root: `C:/dev/perf/warp-sideways-20261024/r26-regression/`.
Executable SHA256 `166389215d08592336ba42727fb0031fb847fb07458b2bf880d420d5d7607f18`.
Hardware NVIDIA RTX5070Ti; actual fullscreen3840×2160 output, DLSS Quality.

| Matched texture-cache run | Disabled | Cold | Warm |
| --- | ---: | ---: | ---: |
| Renderer constructor |3593ms|3953ms|1284ms|
| Albedo array |2774ms|3015ms|576ms|
| Normal array |202ms|296ms|87ms|
| Graphics worker |6205ms|6527ms|3873ms|
| Startup through first scene |18871ms|19459ms|16567ms|
| Whole process |26734ms|26601ms|23858ms|

Warm38/38 layer hits, zero misses/corruption/I/O errors. Cold4 reused duplicate
layers and built34;492MB written. Warm read531MB including duplicate layers.
This is one matched sequence, with OS/driver caching and timing variance; the
strong result is avoiding repeated mip construction, not a claimed FPS improvement.
CPU tests verify byte-exact mip hits for16 source/format/alpha/RGB combinations,
every key input, corrupted/truncated entries and concurrent Windows publication.
Actual DLSS PNGs are not pixel-identical: disabled versus warm RGB mean absolute
difference0.008/255, p99zero, maximum30. Do not call screenshots bit-exact.

Same scene, frame120, warm cache: native PNG took2991.88ms for readback/encoding
and13,813,153 bytes; JPEG1600 took73.94ms and483,571 bytes. It is a deliberately
lossy preview, unsuitable for judging individual4K pixels, fine shimmer or exact
AA. Rendering remains3840×2160. Fixture verifies all four RGBA/BGRA UNORM/sRGB
formats, brightness, all HUD corners, no upscaling and bounded readback size.

Changed release builds took83.70s and113.76s (latter also builds preview fixture).
These are real code changes, not warmed no-op build costs. Shader/material
iterations should use the existing transactional reload path to avoid recompilation.

Four-player route: two sections,179 clean GPU samples each; covered GPU median
44.53/45.47ms, p9547.16/46.68ms. Dominant passes: raster14.5–16.3ms,
grass copy/nearest8.6–9.9ms, shade7.4–7.6ms. CPU/GPU times overlap and must not
be added. **120FPS/8.33ms is not achieved**, including with this DLSS configuration.
The older `scenario-noon-001` was window-clamped3840×2019; it is not4K evidence.

## Next work, ordered by current evidence

### Runtime follow-up (r27)

Accepted grass depth prepass: identical four-player 3840x2160-output, DLSS
Quality route, 240 frames/run with 60 warmup and the final native PNG excluded.
ABBA covered-GPU medians were 44.529/35.538/35.603/44.578ms. Grass copy/nearest
fell from 9.89 to 1.02ms. This is a visibility/depth optimization, without
changing grass density, distance, alpha threshold, motion or material lighting.
Repeated baseline and baseline/candidate PNGs both had RGB mean absolute
difference about 0.033/255 and p99 1/255; all four car/camera traces matched.

Accepted exact-zero shading early exits: separate ABBA covered GPU
35.631/35.184/35.241/35.601ms; shade 7.568/7.326/7.389/7.574ms. Skip only
contributions whose existing smoothstep/multiplicative weights are zero.
This smaller gain must not be described as a new lighting approximation.

Rejected for default use: instance broadphase (its extra ~0.347ms outweighs
the measured cull saving) and large-first draw order (no meaningful raster
gain, still ~14.5ms). Both experiments remain explicitly opt-in. Native GPU
broadphase fixtures matched actual source IDs, counts and pixels, including
70,003 visible instances and four views; equivalence alone is not speedup.

The new debug readback initially selected the shared shadow target when asked
for camera view 0. Its fixture exposed this; it was not a frame-profiler error.
Diagnostics now use explicit active-view snapshots with submission identity,
and reject stale or no-longer-active views. Final fixture validation is recorded
with the corresponding build, separately from the earlier v2 fixture.

Evidence: `C:/dev/perf/warp-sideways-20261024/r27-runtime/` contains each run,
raw timing, manifests and native captures. These controlled route results do
not establish typical gameplay FPS. The 120FPS/8.33ms target remains unmet.

### Startup follow-up

The user's ~22-second "Preparing graphics" wait was not a texture-cache miss.
The measured renderer constructor took 21.877s: shade pipeline 10.815s,
transparency 3.170s, grass pipelines 2.800s and far-grass pipelines 2.509s;
albedo/normal arrays together took only 0.671s. Warp was not passing a pipeline
cache to wgpu. Persistent Vulkan pipeline caching is now implemented and measured;
texture caching alone must never be presented as solving total startup.

Matched same-executable four-player route, textures already warm:

| Measurement | Pipeline cache off | Empty application cache | Warm application cache |
| --- | ---: | ---: | ---: |
| Renderer preparation |23.550s|23.362s|1.361s|
| Preparing graphics, including DLSS |26.051s|25.819s|3.860s|
| Whole scenario startup to first scene |45.047s|45.033s|22.630s|
| Steady covered GPU median |35.326ms|35.251ms|35.285ms|

Warm cache loaded1,603,120 bytes, zero cache errors, unchanged blob at renderer
savepoint. Driver cache hit counts are unavailable and remain null. Native PNG
off/warm RGB mean absolute difference0.03274/255, p99 1/255, matching repeated
DLSS runs; all four camera/car traces and lighting matched. Evidence is
`r27-runtime/pipeline-cache-comparison.json`. This is one controlled sequence,
not a universal launch-time guarantee. New shader variants still need compiling;
unsupported backends and invalid/incompatible caches compile normally.

The accepted single-player playtest manifest is
`build/dev-runs/20261024T194651269Z-cd17a581/iteration.json`: graphics3.61s,
whole startup16.81s, no-op release build1.88s. Changed release builds in this
round took83.86/84.34/101.25s (see exact recorded timings); no-op build timing
must not substitute for them. Next largest restart cost is CPU stage/visual
construction and device/DLSS setup, not warmed shader compilation.

Scenario manifests now record bounded hashes of the 13 supported disk shader
overrides before/after each run, with explicit embedded fallback. Reports group
by shader contents as well as executable/settings, and flag changing or
unverified overrides. Pre/post snapshots cannot prove which live-reload
generation was accepted or detect edits reverted during a run.

### Remaining priorities

1. Cache/reuse static generated stage data: initial visuals~7s total; distant
   forest alone~2–3.4s, terrain/road~1–1.6s. Key exact seeds, phenotype, detail
   settings, assets and algorithm version. Keep mutable physics/tracks and
   camera grass residency outside the static cache.
2. Keep a scene/device alive across more scenario edits; device creation and DLSS
   preparation remain substantial per-process costs. Do not pretend a pipeline
   cache removes those costs. Material/shader reload and in-scene sun changes
   already reuse resources.
3. Improve GPU raster/grass costs using measured sections. Tree archetype/CLOD
   generation is only~25–43ms CPU and is not the current startup bottleneck.
4. Make F12→scenario generation strict and explicit about scene-camera replay
   versus complete input replay. Improve shader-change detection beyond maxmtime.
5. Measure an optimized iteration build profile before switching defaults; retain
   release builds for acceptance. Track build, load, capture and frame cost separately.

Actual next-stage GPU smoke passed round1→2; loader remained responsive with567
loading frames, worker10.12s, and no physics advancement during initial load.
It was1280×720 native diagnostic validation, not a performance benchmark.
