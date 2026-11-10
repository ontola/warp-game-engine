# Packed seating in the Growing Guns match

9 November 2026. Release build based on c032ee7a, Windows, RTX 5070 Ti,
Vulkan, full graphics, native 3840 × 2160. Seed 52 seats 6,638 spectators
in the standard layout and 10,700 in the packed layout. The reserved pool
holds 15,000 in both cases.

Each run rendered the same scripted match, including gameplay and effects,
for 30 warmup frames and 120 measured frames. Four-player runs used three
matched pairs, alternating which layout ran first. For single player, one
complete pair is included. Further measurements were stopped once another
interactive GPU application opened.

| Views | Layout | Spectators GPU ms | Whole frame GPU ms | Serialized FPS |
| ---: | --- | ---: | ---: | ---: |
| 1 | Standard | 0.287 | 6.18 | 91.6 |
| 1 | Packed | 0.419 | 6.30 | 90.6 |
| 4 | Standard | 0.307 | 8.39 | 47.7 |
| 4 | Packed | 0.557 | 8.63 | 45.3 |

The spectators column uses the mean of the summed per-view Warp Humans
pass from each run. Whole-frame GPU and serialized FPS use medians within
each run. Four-view values are medians across runs. Serialized FPS waits
for GPU completion every frame; it varied from 44.3 to 63.8 over the
four-player runs. The cameras follow the players on the field. A separate two-view crowd-only
fixture (960 × 540 per view, one camera among the seats) took 0.711 ms
standard and 0.969 ms packed across three runs each. The close camera
selected 53 and 79 high-detail people respectively.

Reproduce with `bench --seed 52 --graphics full --warp-humans --views 4
--size 3840x2160 --frames 120 --crowd-density packed`. Repeat with
`--crowd-density standard`. [Numbers for each run](../evidence/warp-humans-density.json).

## Receiving the stadium lights

A second comparison kept the 10,700 packed spectators and changed only
their lighting. Replay the in-stands view with `shot --seed 52 --views 1
--size 1280x720 --warp-humans --start-in-stands --seconds 0 --sky night`.
The floodlights reveal shirts and faces on that tread. At noon, the sunlit
front rows and the shaded rear rows follow the edge of the roof.

The existing GPU selection dispatch samples each visible person's upper
torso against the sun shadow and active local lights. A shadowed spotlight
uses one comparison in its existing map. Clothing and skin respond to the
summed light and its mean direction. The sample is shared over the body,
so shadow boundaries crossing one person resolve at torso height.

With `bench --game-loop --seed 52 --warp-humans --graphics full
--size 3840x2160 --frames 105`, two pairs in old/new/new/old order per sky
(after 30 warmup frames), the mean spectator GPU time went from 0.447 to
0.537 ms at noon and from 0.443 to 0.541 ms at night. The median full frame
GPU time per night run ranged from 9.07 to 9.09 ms before and 9.16 to
9.19 ms after. These timings use one player's camera.

The fixed daylight four-view benchmark above also ran twice per lighting
version with the packed layout. The spectator pass measured 0.521–0.522
ms before and 0.466 ms after, with 8.39 and 8.34 ms median whole-frame GPU
time respectively. These small differences depend on the camera and pass mix.
