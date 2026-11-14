# Rainforest camera selection and splat raster

The Sideways rally capture in [the RTX 5070 Ti review](https://github.com/ontola/warp-game-engine/pull/54#issuecomment-6863251205) took 27.8 ms GPU at 2560×1440 on `cb17f3d9`. The voxel generation and raster passes took 14.9 and 4.1 ms. The capture reported 7.97 million splats and zero source slots in use. Main subsequently changed the per-tree source cut, so the baseline for this change combines biome commit `85b628ce` with main `264b4e2e` (tree `81a4c0554d804eae63d0b38bad56684bba6d0eca`).

## Geometry checks

On the software Vulkan adapter, a shadow-only fixture previously requested both source pages for a branch that would always cast a voxel shadow. The new cut requests zero. With a close camera it requests both pages, retains a complete voxel parent while streaming, and publishes both source siblings when resident. A separate unary chain failed to reach source when its intermediate voxel required 200 splats against a budget of six. With a resident mesh costing three clusters the new camera cut draws one source record and the shadow retains its voxel root.

Rasterizing 48,000 overlapping splats with the old six corners and the indexed four corners produced identical visibility IDs, splat IDs and depth bits on Mesa llvmpipe. The camera covered 14,749 pixels. The sun covered 24,643 pixels. The cached sun raster also matched with partial tile damage in both dither modes (12,573 and 12,649 pixels).

The CPU build of each 4×4×4 brick now records the bounds of its occupied cell centres. Mean projected rectangle area, averaged over the three orthographic axes with raster padding included, shrank by 32.3% for kapok, 23.3% for African mahogany, 30.8% for raffia palm and 34.5% for Scots pine. These percentages measure culling bounds. The number of splats an actual camera rejects depends on the depth in that view.

An optional subgroup append path made 27 global reservations for 132 surviving splats on llvmpipe. The scalar path needs 132. Readback verified payloads when all fit and checked bounded writes when only 13 or zero records fit. Use `KERN_SPLAT_SUBGROUP_APPEND=1` to time this path independently on hardware. It is off by default.

## Sideways held-camera comparison

The before and after release binaries rendered seed 1 at 120 m on the same 1 km rainforest stage, 384×216, clear weather, wind 0.3 at time zero, eight TAA frames, one player, Mesa llvmpipe (LLVM 21.1.2). Both had 189,210 vegetation roots. The two PNG files have the same SHA-256 (`c7dcdf09d9a2d102a235bfb82071be66f7f4ef6b8d64a6346b7646b6abbb56b6`). At this resolution and frame count the source cut was identical. Hardware timing and steady-state quality at 1440p remain open.

| Final camera frame | Before | After |
| --- | ---: | ---: |
| Source slots / booked source clusters | 6 / 3,574 | 6 / 3,574 |
| Voxel slots | 5,685 | 5,685 |
| Submitted splats | 191,160 | 191,256 |
| Distinct input vertices | 1,146,960 | 765,024 |
| Splats that won at least one pixel | unavailable | 11,378 |
| Pixels with a splat ID | unavailable | 28,653 |

All submitted splats fit. In the new profile 5.95% of them reached the final splat ID target and 123 submitted splats had diameters above 8 px. Both revisions counted 4,490 over-target camera proposals with missing pages. The old diagnostics included the brick retest queue in the camera capacity. The new value of 5,786,176 correctly excludes the queue from the unchanged allocation of 6,612,772 slots.

The render release unit suite passed 934 tests with 88 marked ignored. The source-cut regressions were also explicitly run on llvmpipe and failed as expected when only the old shader was substituted. There is no RTX timing for this change. The card was in use during verification.

## RTX reproduction

Compare the two revisions with the same driver, warmed page cache, wind, viewport and frame count. For the existing F4 captures, use `terrain_fly --seed 1 --style rainforest --from <capture.json> --fly 300 --stationary --size 2560x1440 --upscale native --fly-times <times.json>`. Compare the final 200 frames and save a still at each pose. These `--at` values also reconstruct the cameras without a capture file. Their third component preserves the camera's height over each biome's terrain.

| Capture | `--at X,Z,ABOVE,YAW,PITCH` (headings in radians) |
| --- | --- |
| `terrain-fly-1794249228953.json`, near | `-121.3255157,-5596.477539,6.3668213,3.944799,0.062000` |
| `terrain-fly-1794485139523.json`, overlook | `389.0228577,-573.3994751,48.2914429,0.419595,-0.415399` |

Repeat for Norway and for Swiss with `--spruce-forest`. For the game camera use `sideways shot rally.png --biome rainforest --seed 1 --km 1 --at 120 --size 2560x1440 --taa 8 --wind 0.3 --wind-time 0 --voxel-raster-profile`. Record pass times, camera and shadow budget use, source slots, stored and winning splats, and visible changes at the tree line and near the road.
