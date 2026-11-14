# The green grain on the pendant shade's inside: SSR noise (open), 14 November

Main at fe5e8f3e, Mancave (RTX 5070 Ti, Windows), `interior_rooms`, idle
GPU, 1920x1080, EV 6.3 fixed, fog off unless said. Joep's F4 camera of 12
November (0.458,1.65,-8.299, yaw -15.859, pitch -0.230), the pendant at
its pose after 215 frames, swinging or held there (`ROOMS_HOLD_T=3.5833`).
Grain: mean |luma - mean of its 8 neighbours| (0..255) over the shade's
inside right of the bulb (915,150,85,90). Corrects
[2026-11-14-pendant-shade-grain](2026-11-14-pendant-shade-grain.md),
which measured with the fog on only.

## What it is

| | grain |
| --- | --- |
| held | 1.07 |
| swinging | 1.21 |
| held, `KERN_TRACED_REFLECTIONS=0` | 0.48 |
| held, `KERN_REFLECTION_PROBES=0` | 1.23 |
| held, `KERN_RSM=0` | 1.11 |
| held, hits with any light-face share left out (diagnostic) | 1.05 |
| held, 96 consecutive frames averaged | 0.60 |
| held, one of those frames | 1.05 |

Noise of the traced reflections, not detail: averaged over 96 frames the
shade is nearly as smooth as without them. One frame differs from the
next by only 0.32 in that crop, so the grain changes slowly: the temporal
pass (a tenth of each frame) and TAA leave noise correlated over about ten
frames. Not the bulb's face (left out: no change), not the probe fallback
or the live bounce.

## Tried, not kept

| change | held | swinging |
| --- | --- | --- |
| the rays' random per frame from the full frame count, not frame % 16 | 1.07 | 1.23 |
| temporal box 3 sigma instead of 1 | 1.02 | 0.98 |
| 16 reuse samples instead of 8 | 0.86 | 1.03 |
| a tenth -> 0.03 per frame (diagnostic) | 0.73 | |
| history length per pixel (mean of up to 32 frames while inside the box, a tenth once moved) | 0.86 | 1.37 |

A longer history converges a still shade but makes the swinging one
worse: it lags, and the noisy 1-sigma box then clamps it to its edge every
frame. The temporal pass cannot tell where a long mean is safe without the
reflecting surface's own motion.

## Next

Per-pixel motion of the reflector into the temporal pass (a motion-vector
pass for opaque surfaces, tried on 12 November for the reprojection only)
to keep long histories on still surfaces and short ones on moving ones; or
temporal moments per pixel (SVGF) for the box instead of the noisy 3x3 of
this frame.
