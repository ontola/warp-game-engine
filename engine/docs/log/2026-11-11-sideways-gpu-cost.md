# Sideways GPU cost, 11 November 2026

Machine: M5 Pro (Metal), headless `sideways shot --size 2560x1440 --taa 64`,
default seed and position. Frame time is (wall - first frame) / 63; the Mac
was in desktop use, so identical runs differed by 2-3 ms and only
alternating runs count. Commits: main from 885bf1f9 to bb638564.

## Where the frame goes

Two Metal System Traces (34 and 2 TAA frames) differenced per encoder label
give the steady frame: 26.4 ms GPU at one view.

| Pass | ms |
| --- | ---: |
| raster (all 32 passes: view, near and shared sun maps, occluders) | 7.7 |
| shade | 6.7 |
| cull | 5.9 |
| shade terrain | 2.1 |
| taa | 0.74 |
| tonemap | 0.48 |

The scene has 268,585 instances: 32,017 near trees and 219,744
distant-forest trees, all shadow casters.

## Geometry LOD error 2 px (cc9e99aa, default)

| `--lod` | ms (two rounds) | view tris | shadow tris | vs 1 px |
| --- | --- | ---: | ---: | --- |
| 1 | 29.8 / 32.3 | 6.39 M | 17.9 M | |
| 1.5 | 28.0 / 30.1 | 4.81 M | 14.1 M | 53.5 dB, 114 px > 2 % |
| 2 | 26.3 / 29.1 | 3.84 M | 9.7 M | 49.5 dB, 303 px > 2 % |

## Existing switches

- `KERN_INSTANCE_BROADPHASE=1`: identical image, no gain in alternating runs
  (+0.1 to +0.6 ms at one view, -0.45 at two). Stays off.
- `KERN_STATIC_SUN_CACHE=1`: alone it caches only 2,098 non-tree casters;
  frozen trees need `KERN_SUN_CACHE_GUARD_METRES` > 0. With 20 m: 253,104
  static and 765 dynamic casters, 1.27 M dynamic tris, but at most ~0.3 ms
  over the broadphase run in the same window; 49.6 dB (shadow edges move with
  the guarded anchor).
- Instance groups (`KERN_INSTANCE_GROUPS`, `KERN_SUN_INSTANCE_GROUPS`,
  `KERN_HEADLIGHT_INSTANCE_GROUPS`): 4,197 leaves under 525 parents; cull
  passes fall to near zero (near shadow cull 1.0 -> 0.0 ms, occluder cull 1.8
  -> 0.2 ms), identical image. Whole-frame results on the Mac flipped sign
  with load. The Mancave measured 2 views 11.9 -> 9.9 ms and 4 views 16.6 ->
  12.9 ms (DLSS), and bb638564 made them Sideways' default.

## Shade: cars

Compiling features out of the shade constants one at a time (median of three,
shade pass / frame ms): base 7.1 / 22.2; cars 4.9 / 18.8; grass 6.1 / 20.5;
every other feature 6.7-7.1. Inside the car cost: the clear coat block about
1.0-1.1 ms (its traced reflection 0.4-0.7 ms), the procedural car_surface
layers about 1.0 ms together, reflected_visibility 0-0.2 ms.

Rejected:
- A separate car shade class (cars compiled out of the rest): rest 7.1 ->
  4.2 ms, but the car pass costs 2.7 ms; frame -0.1 to -1.0 ms. Windows,
  plates and lamps need the instance category, not the material tag.
- Skipping fully faded `car_fbm` octaves: bit-identical image (104 dB), gain
  within noise.
- A car shading LOD by pixel footprint (2/4/8 mm): -0.2 to -1.5 ms at one
  view, nothing at two, and 700-1,550 px changed even at 8 mm.

## Shadow distance LOD (`KERN_SHADOW_DISTANCE_LOD`, off)

Trees are over 99 % of shadow triangles (static casters: 80k tris). The
shared sun map refined every caster to ~2 shadow texels wherever it stood.
With the switch, a caster's error may grow to F x what one screen pixel of
the nearest camera spans at its distance; it is evaluated per bounding
sphere, as perspective views are, so parent and child cuts agree.

| F | shadow tris, 1 view | image | shadow tris, 2 views | image |
| --- | ---: | --- | ---: | --- |
| off | 9.69 M | | 10.15 M | |
| 1 | 5.31 M | 80 dB, 0 px > 2 % | 3.45 M | 67.5 dB, 1 px |
| 2 | 2.98 M | 67 dB, 1 px | 2.61 M | 62 dB, 3 px |
| 4 | 2.15 M | 63 dB, 2 px | 2.42 M | 51 dB, 346 px |

Frame time on the Mac did not move measurably (shadow raster is a small share
there). The Mancave, where shadow raster is 2.3 ms at two views, is to
measure F = 1 and 2 before it becomes a default.
