# Retained terrain forest resources, 3 November 2026

## Scope and acceptance

Experimental and opt-in: `WARP_TERRAIN_STREAM_GROUND=1` plus
`WARP_TERRAIN_STREAM_FOREST=1`. Brick HZB culling remains disabled. This is the
first step toward removing movement hitches, not a claim of 120 FPS or completed
Nanite-style foliage.

Previously every 300 m forest recenter built a whole Scene and Renderer,
including immutable textures, geometry, pipelines, view resources and histories.
The new path computes placements and baked sun visibility on a CPU worker, then
updates reserved proxy instances and the existing heightfield texture. The
painted far-forest handover moves to the same centre. Near assemblies continue
to use their existing movable slots.

The demo reserves 50 percent extra proxy slots and 25 percent scale headroom
for full assembly slots. This is capacity, not additional visible forest. The
extra reserved records still have a culling/scan cost. Spare proxies park far
below the world. Replacement updates owner transforms, canopy revision, shadow
invalidation and voxel transforms together; temporal history is reset because
volume IDs may now refer to different trees. Branch geometry and voxel asset
catalogs are retained.

Capacity exhaustion, unseen species, insufficient trees of a species to fill
its full slots, or exceeded scale headroom return a no-write fallback and build
a full renderer. Seed, biome and graphics-content changes still rebuild. Fir
fallback scenes and scenes without a recipe pool continue to use the old path.

## CPU ownership index

`KERN_OWNER_RANGE_INDEX=1` replaces linear immutable ownership scans with a
sorted-index lookup in voxel forests and part wind. Terrain patch updates used
to search all forest/part owners repeatedly to reject writes to GPU-owned
objects. Index construction is once per renderer, not once per frame. The
original volume order remains unchanged for shaders and proxy validation.

The first retained-resource experiment did not improve route wall times. Slow
frames also occurred before any recenter, at terrain patch swaps. Increasing
reserved owner capacity amplified those scans. The index is measured separately
from retention; neither result should be attributed to voxel raster culling.
The CPU lookup property test covers holes, duplicates, empty ranges and
saturating end bounds. Canopy tests also pass (13 tests).

Streamed proxies now recompute the canopy's fixed-point mass budget and height
envelope, with the original finite-world and per-crown work limits. Reserved
inactive slots are excluded from canopy density. The first GPU smoke run
correctly rejected changing crown scale against the old immutable budget; that
was repaired, rather than bypassing the safety check.

## Final measured result

Same executable, three alternating runs per mode. Values below are medians of
run statistics, not pooled percentiles. One native 1713x964 view. Wall samples
include GPU completion/readback; they are not gameplay FPS or pure CPU work.

| Mode | Wall median | Wall p99 | Wall maximum | GPU covered median | Terrain CPU median |
|---|---:|---:|---:|---:|---:|
| Linear guards, rebuild | 56.64 ms | 519.91 ms | 622.04 ms | 10.06 ms | 36.66 ms |
| Indexed guards, rebuild | 45.77 ms | 104.29 ms | 475.53 ms | 9.83 ms | 26.42 ms |
| Indexed guards, retained forest | 45.20 ms | 107.30 ms | 121.04 ms | 10.11 ms | 25.12 ms |

Frames over three times each run's median: baseline 38/36/34, index-only 4/1/1,
combined 0/0/0. Indexing is the main win; retaining resources removes the remaining
large rebuild outliers on this route, without claiming a steady GPU speedup.
The absolute baseline changes with background load; keep the same-executable,
interleaved comparisons together. Async worker completion can occur at different
simulation frames across modes, so this is a whole-route implementation result.

One stationary index-off/index-on PNG pair is pixel-identical below the top
180 HUD pixels. Moving captures were inspected; terrain and forest remain
visible after recentering, but lighting/noise quality still needs work. This is
not raw visibility/depth equivalence or full wind/species/split-screen coverage.
The current final executable hash and per-pass, per-run CPU/GPU statistics are
in [the evidence JSON](../evidence/terrain-forest-streaming-2026-11-03.json).

All changes remain opt-in pending acceptance. No interactive window was opened.
The 8.33 ms target is not reached. Next priority is the CPU terrain selector and
patch update path (about 25 ms even with indexed guards); only optimizing voxel
GPU rasterization would leave that CPU limit intact.

## Remaining work

This is not yet a chunked streaming system: it replaces the complete reserved
proxy instance range at a recenter. Placement generation and the baked sun field
are also recalculated in full on the worker. Meadow still uploads its full
23 MB ground field. Budgeted deltas, preserving overlapping tree IDs, avoiding
the temporal reset, and incremental canopy/shadow invalidation remain useful.
The goal is to eliminate large frame spikes before optimizing steady near
geometry raster cost. Those are different measurements.

## Reproduction

Release `terrain_fly`, same executable for both modes. Ground streaming on;
forest streaming alternating off/on, brick HZB off. Alpine seed 1, grass and
track, `--fly 240 --speed 240 --size 1713x964` (one view). Three pairs.
`--fly-times PATH` stores frame wall samples after the initial five frames;
`WARP_FRAME_LOG` supplies asynchronous GPU pass times separately. `--fly-shot`
stores the last frame outside the frame wall measurement. These headless wall
times include readback, and are not presented gameplay FPS. Background load
from the user's film is uncontrolled.

The final comparison is three repeats of each of: linear queries/rebuilds,
indexed queries/rebuilds, and indexed queries/retained forest. These nine runs
share an executable. CPU game draw, terrain update, tree pool update, renderer
submission, HUD, and renderer CPU spans are recorded separately from GPU times.
The `--fly-times` sidecar `.cpu.json` contains these raw CPU samples. GPU logging
is re-enabled on the actual Renderer after swaps, even if its address is reused.

Earlier six-run retention-only evidence: `forest-stream-v2-*` (not accepted).
Final comparison prefix: `forest-index-v3-*`.

Local evidence: `C:/dev/warp-bench/forest-stream-{0..5}.log`, `.jsonl`,
`-times.json`, and representative `forest-stream-{0,1}.png` captures.

Verification: relocation unit test checks local voxel coordinates, owner/asset
identity, conservative transformed bounds and owner transforms across rotations
and scales. The nine GPU route runs completed without validation errors. All three retained-mode runs applied updates without capacity fallbacks. The transformed-proxy unit test, ownership query test and 13 canopy tests passed.
