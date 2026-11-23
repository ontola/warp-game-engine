# Port Meridian planting and download size

Release source `9b20efce` places 25 broadleaf trees and 27 flowering shrub beds along Exchange Avenue, behind residential blocks and in the parks. The city still uses its 27 shared maritime pines. The eight pines on the two park lawns and their grates sit beside the diagonal walks. Grass is cut back under the planting beds. The new trees have narrow trunk colliders and the foliage leaves space above the walking routes.

Two reusable, opaque glTF models supply the new planting. The tree is 26,040 bytes and the shrub bed is 24,828 bytes. All other copies are placements generated locally from the city seed. Both models are texture-free. Their Python baker is development source. [Plant baker](../../../games/warp-theft-auto/scripts/bake_plants.py)

## Rendering cost

A Windows RTX 5070 Ti rendered seed 1948 at 1600 x 900 using the same native runtime before and after the game change. Two warmed before/after pairs sampled 120 frames per scene per build per run. The figures below are GPU hardware timestamps in milliseconds. All samples completed, with no timestamp drops.

| View | Median before | Median after | p95 before | p95 after |
| --- | ---: | ---: | ---: | ---: |
| Exchange Avenue | 2.248 | 2.299 | 2.469 | 2.575 |
| Moving through Lido Gardens | 2.518 | 2.518 | 2.768 | 2.709 |
| Skyline | 2.136 | 2.172 | 2.405 | 2.396 |
| Two-player avenue and garden | 3.188 | 3.233 | 3.514 | 3.556 |

The largest median GPU increase in these views was 0.051 ms. Serialized end-to-end median frame times changed by -0.09 to +0.08 ms. Browser and lower-end GPU performance will depend on the device.

## Browser transfer

On the public shelf `955f2ad2-20261123094456`, Edge transferred 19,630,261 bytes on a cold visit, compared with 48,131,892 bytes in the preceding planting-free build. A player updating from `b0ff614d` transferred 52,726 bytes including headers. The same browser revisiting the new build transferred 274 bytes. Both returning sessions reused the engine with zero engine bytes transferred. The two new vegetation models accounted for 18,106 compressed body bytes on the wire.

The reusable packaging command prints a per-release byte report and accepts an optional report path outside the shelf. [Browser packaging and cache keys](../browser.md)
