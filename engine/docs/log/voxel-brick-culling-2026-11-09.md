# Voxel splat brick culling, two passes, 9 November 2026

Replaces the 3 November prototype ([voxel-brick-culling-2026-11-03.md](voxel-brick-culling-2026-11-03.md)),
which only let trees within 150 m occlude the rest, and did not run with paged
foliage assets (the swiss forest always pages). On by default;
`KERN_SPLAT_BRICK_HZB=0` turns it off.

## Why

Mancave, terrain_fly swiss spruce forest: only 8% (Joep's view, 1440p) and 17%
(valley, 4K) of the stored camera splats won a pixel, and making the splats
cost 6.5 / 8.2 ms against 2.2 / 3.0 ms raster.

## How (voxel_forest.wgsl, VOXEL_BRICK_CULL)

1. `voxel_splats` tests every 4x4x4 brick of a tree's chosen level against the
   previous frame's reversed-Z minimum pyramid, at the view-projection it was
   built with (camera header records 4-7, record 8.x = 1 once valid). Lanes
   test 64 bricks at a time, then split the visible bricks' cells. Hidden
   bricks are queued at the end of the splat records (an eighth of them); a
   full queue splats the brick now.
2. The first raster draws. Its depth becomes this frame's pyramid.
3. `voxel_splat_cull_base` makes the second pass start after the first's
   records (header 11.z) and sizes its dispatch; `voxel_splat_retest` splats
   the queued bricks the new pyramid does not hide; `voxel_splat_cull_rest`
   writes the second draw's arguments (stats words 26-29) and keeps this
   view-projection for the next frame.
4. The second raster draws only the new records.

Stale history only costs the retest, so no brick is lost. Paged assets use the
page cache's own brick descriptors (`VOXEL_PAGED_BRICKS`).

## Measured (Mancave RTX, 5b534e5, off -> on)

| | Joep view 1440p | valley 4K |
|---|---|---|
| stored splats | 4.44M -> 3.35M | 5.93M -> 5.00M |
| queued bricks / retest splats | 2.45M / 49k | 763k / 14k |
| splat generation | 6.54 -> 3.76 ms | 8.17 -> 7.06 ms |
| raster (both passes) | 2.21 -> 1.66 ms | 2.96 -> 2.49 ms |
| pyramid + cull | 0.36 ms | 0.24 ms |
| GPU frame | 35.50 -> 32.55 ms | 39.32 -> 37.71 ms |
| winning splats | identical | identical (±0.2%) |
| image mean abs diff (8-bit) | 0.06 | 0.08 |

Fast turns and forward flight: no missing trees, frame-to-frame change the same
as off. Pale coarse discs on trees re-entering view during a fast turn also
happen without culling: the refiner's camera splat limit refuses them for a
few frames.

## Limits

Most hidden splats sit behind splats of the same tree and brick, which a
per-brick test cannot remove, so stored splats drop 16-25%, not 90%. The
refiner did not refine further (cut and refused refinements unchanged): the
branch slots, not splats, still bind for near-pixel voxels.
