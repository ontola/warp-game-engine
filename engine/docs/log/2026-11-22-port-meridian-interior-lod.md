# Port Meridian ground-floor LOD

The previous public playtest (`c1148402`) represented an unloaded ground floor with an opaque box. A walker arriving within 4.5 m replaced the entire box with a furnished room. The replacement affected every camera. We measured the change through the same office window while approaching, reversing, and walking away.

The engine gained view-masked coverage for opaque Luau primitives in `ed4b0e8e`. The GPU writes either proxy geometry or the depth and material behind it at each sample, using a stable screen-space pattern. Ordinary opaque geometry keeps its existing raster path. Port Meridian uses the room’s seed, dimensions, mirror, upholstery and wood to shade a coarse interior on the existing shell. Only a visible opening evaluates the ray-box intersections. Actual furniture, glass, occupants and architectural lighting remain close-range resources. A four-player test with four furnished rooms used 1,175 of the 1,280 entity slots.

## Visual comparisons

The native RTX 5070 Ti replay used seed 1948 and a 960 × 540 output. We tracked a physical patch of the window across the change in camera position and measured the mean absolute pixel luminance change on a 0–255 scale. A second camera stayed 23 m away for the split-screen test.

| Window sample | Previous arrival | Released arrival | Previous departure | Released departure |
| --- | ---: | ---: | ---: | ---: |
| Civic Center office, daytime | 45.64 | 0.21 | 45.67 | 0.25 |
| Same office, stationary second player | 46.88 | 0.06 | 44.64 | 0.08 |
| Lido Gardens hotel, 22:30 | 33.04 | 0.33 | 27.58 | 0.55 |

At 14 m, before streaming the office, three consecutive 0.8 m sideways steps each shifted the reception monitor 0.29 m relative to the window frame. This matched its projected depth. The masonry stayed registered to the facade.

The initial GPU transition still showed a night flash. The room reflection probe extended all the way to the exterior wall, so the renderer treated its outward-facing proxy as part of the lit lobby. The hotel arrival changed the sampled window by 51.26. Turning off reflection probes removed the jump. Setting the volume boundary at the physical glazing fixed it with the probes enabled. Six metres inside the finished hotel, rear-wall brightness changed from 71.45 to 72.49 and ceiling brightness from 107.07 to 107.02.

In the published WebGPU build `34a8d24e-20261122225950`, a continuous keyboard walk past the office had mean window changes of 0.31 on arrival and 0.25 on departure. The old browser build measured 45.63 and 45.97 over the same transitions. At the night hotel, the new browser measured 0.64 and 0.55. The test used actual W/S input with the fixtures, reflections and room streaming enabled.

## Frame time

Four alternating native runs used the same RTX 5070 Ti with the browser closed. Each run measured 120 warmed frames per position at 960 × 540. The figures are the average of the two per-build medians and 95th percentiles, in milliseconds.

| Distance to opening | Previous median / p95 | Released median / p95 | Median post-submit GPU wait, previous / released |
| --- | ---: | ---: | ---: |
| 16 m, distant room | 5.27 / 6.53 | 4.88 / 6.46 | 1.99 / 2.01 |
| 3.25 m, both levels visible | 7.48 / 9.15 | 7.01 / 9.44 | 2.62 / 2.64 |
| 0.5 m, fully revealed doorway | 7.56 / 9.28 | 7.99 / 10.09 | 2.61 / 2.65 |

[Game controls and streaming](../../../games/warp-theft-auto/README.md) · [Luau rendering API](../warp-cli.md) · [Room lighting](../interior-light.md#streamed-ground-floors-in-luau)
