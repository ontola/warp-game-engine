# Terrain Fly performance, 21 November 2026

Baseline `35736fde`. The November 19 hardware flight pointed to periodic terrain
planning, outer sun shadows and forest refinement. The replays ran on Mancave
with a Ryzen 9 5900X. The full routes used an RTX 5070 Ti with Vulkan. The
isolated shader comparisons used the D3D12 Microsoft Basic Render Driver.

## Moving frames on the RTX 5070 Ti

Both builds ran the same 300-frame headless routes at 2560 × 1440 native,
Swiss seed 1. The spruce walk started at `-21,-116,1.7,0.4,0` and moved at
4 m/s. The mixed-forest flight started at `-400,300,80,2.4,-0.4` and moved
at 40 m/s with grass enabled. GPU times cover the timestamped passes. Wall
times include CPU rendering and a blocking RGBA readback on every frame.

| Frames 101–299, milliseconds | Before p50 | After p50 | Before p95 | After p95 | Before p99 | After p99 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Spruce walk, GPU | 17.24 | 16.73 | 19.55 | 19.10 | 20.49 | 19.51 |
| Spruce walk, headless wall | 29.82 | 29.44 | 42.05 | 34.56 | 45.67 | 36.18 |
| Mixed forest and grass, GPU | 13.41 | 12.79 | 23.39 | 19.61 | 50.02 | 49.42 |
| Mixed forest and grass, headless wall | 34.69 | 29.32 | 44.56 | 37.29 | 70.53 | 69.66 |

The forest selection and residency pass fell from 2.52 to 1.96 ms on the walk
and from 2.70 to 2.21 ms on the flight (medians). The GPU allocator reported
208 MiB less on both routes. Final frames had zero camera fallback roots and
zero refinement faults. The mean absolute difference between matching
end-frame images was below 0.012 on the 0–255 RGB scale. Streamed populations
can settle at different ticks. The deterministic comparisons below check
individual placements, selected records and terrain geometry separately.

Each table row uses one run per build. The walk was repeated on the updated
build after the baseline compile finished. Its first run during compilation
had a 16.81 ms median GPU interval and a 41.66 ms median wall time.

Four flight frames (138, 192, 238 and 276) exceeded 33.3 ms on the GPU in
both builds. The outer voxel shadow raster alone took 29–35 ms on those frames.
The worst GPU interval was 56.62 ms before and 56.56 ms after. A separate
278-frame replay captured the per-level projections and dirty masks. On
each spike, every one of the eight sun maps redrew all 4,096 tiles. On
frames 138 and 276 the clipmap base scale changed. On frames 192 and 238
the shared depth projection changed. Both events invalidate retained map
depth under the current cache rules.

## Terrain planning

The CPU replay used the production planner and terrain field, Swiss seed 1, a
native 1920 × 1080 camera, sun caster detail and 12 m/s flight. Five paired runs
of 180 frames alternated execution order. They included turns, a teleport and a
second camera. The old and new planners agreed in all 900 plans on the patch
set, stitch masks, displayed slots, adaptive LOD, change counts and geometry
jobs, including the bit patterns of morph parameters.

| CPU planner workload | Samples | Before | After |
| --- | ---: | ---: | ---: |
| Flight selection update, median | 210 | 6.417 ms | 2.618 ms |
| Flight selection update, p95 | 210 | 8.597 ms | 3.443 ms |
| All warmed flight frames, mean | 625 | 2.294 ms | 0.928 ms |
| Turn, teleport and two-view selection updates, median | 140 | 6.172 ms | 2.486 ms |

The planner precomputes each camera cone, balances splits from fine to coarse
and reads stitch flags from the balanced leaves. Small changes reuse unaffected
stitch keys. Morph source lookup indexes the patches retired by the swap.

## Grass streaming

A separate release comparison used the original and updated grass generators
from seed 1 at 17 centres in Swiss terrain. The field radius was 240 m. Each
step moved 12 m in x and 10 m in z, crossing the 15 m streaming threshold.
All 777,586 returned roots matched in order, identity and placement. The first
field took 476 ms before and 363 ms after. Across the 16 subsequent centres,
alternating measurement order, median worker time fell from 552.9 to 22.7 ms.
Terrain-dependent placement evaluations fell from 939,601 to 45,269.
This measures grass generation on the streaming worker. The woody plants,
combined population diff and renderer upload have separate costs.

## Forest selection

The default selector sizes the early and late dispatches independently after
the seed round. They share the original queue capacity. Each retained family
keeps its priority and each rejected family keeps its parent. Camera-only and
shadow-only tasks compute the coverage estimates relevant to their budgets.
At a capacity of 1,048,576 tasks, deferring the alternate selector's 96-byte
and 112-byte proposal buffers saves 208 MiB of allocated GPU memory.

The production refinement shader ran on 8,192 generated roots, one camera
and three shadow views with mixed retained shadow depths. Across 17 warmed
alternating pairs, GPU time fell from 40.59 to 24.53 ms. Each variant tested
34,516 nodes. The full comparisons matched all 26,550 selected records and
the per-root camera and shadow histories. All pairs matched their counters.

## Sun shadows

An outer map with pending damage can move its mask with its retained depth
when the projection translates by whole tiles. The GPU snapshots the source
mask, translates it and marks the exposed strips. Changed casters in the
new frame then mark their old and new bounds in the new grid. Incompatible
moves still trigger a full redraw, as do pending scrolls with map-texel dither.

Frames 1–294 of the archived walk contained 418 outer-level draws. Fourteen
full redraws followed deferred frames and moved exactly one 32-texel tile.
Each is eligible for scrolling and exposes 64 of the map's 4,096 tiles.
Caster damage can increase the actual redraw area. The walk records the map
transforms, so these counts do not establish a GPU time saving for scrolling.

The production caster tracker also omits the diagnostic histogram unless
`KERN_SUN_DAMAGE_REPORT=1`. A separate 17-pair D3D12 software replay compared
the two paths over 32,768 source and 32,768 voxel records. Tracking 512 changed
casters took 1.95 ms with the counters and 1.77 ms without them. Tracking
65,536 changes took 2.72 and 2.35 ms respectively. The replay used zero active
maps to isolate tracking, and all 2,016 diagnostic-bin checks passed.

## Production checks

The release Terrain Fly executable built its normal Swiss world with grass
enabled on seed 1 using `--validate-world`. It produced 100,323 active roots,
2,798 hierarchy nodes and 23 meshes. This path runs the scene builder on the
CPU. The software-adapter shadow fixtures compared dirty masks and retained
depth against a full oracle across positive and negative scrolls, including
mask word boundaries and a change of caster on the scroll frame.

The terrain planner passed nine tests, the Terrain Fly example passed three,
and the six refinement and nine shadow-damage tests passed on the software
adapter. All 26 selection tests passed on the RTX, including split dispatch,
overflow and complete-family retention. The first software run hit the test
helper's 30-second readback deadline in 18 selection cases.
