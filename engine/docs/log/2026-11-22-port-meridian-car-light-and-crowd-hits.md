# Port Meridian car lighting and pedestrian hits

The comparison used v1.1 build `afd9b59a-20261122213851`, city seed 1948, a 960 × 600 output and an RTX 5070 Ti. The camera followed the rear quarter of the same sedan near the ferry. The simulation and renderers ran headlessly.

## Two sources of flicker

Rear-window pixels changed even on a parked sedan with a frozen clock. The glass layer's opacity caused temporal antialiasing to discard its previous samples regardless of whether either surface had moved. The renderer now compares the screen motion of the glass and the surface visible through it. Stable layers reuse those samples. The 99th percentile of frame-to-frame brightness change in the daylight rear-window sample fell from 27.3 to 2.3 on a 0–255 scale.

The sunlit trunk showed a second, larger change while driving. The close shadow layer updated every other frame, so the moving car sometimes sampled a shadow of its own previous position. The close layer now refreshes for an affected view when a changing caster intersects it. The outer shadow schedule and the continuous sun clock continue to run.

| Native comparison | Baseline | Corrected | All shadow layers refreshed every frame |
| --- | ---: | ---: | ---: |
| Moving trunk, mean absolute per-pixel brightness change | 17.682 | 0.158 | 0.159 |
| Moving trunk, 99th percentile per-pixel change | 44 | 3.33 | 3.33 |

These figures cover 20 successive frames of the same drive ending at 8.106 m/s. A separate browser reproduction used actual F and W input, captured WebGPU frames and measured a polygon on the trunk at 5.5–9.3 m/s. On the previous public build the mean change of the *whole panel's average brightness* was 9.19. The adjacent roof measured 0.038. The panel-average statistic differs from the per-pixel figure above.

## Shooting people in the street

On the previous public build a normal mouse click aimed at an unmarked park resident produced a gunshot and zero registered hits. Earlier checks had aimed at red-marked rivals, the only free-roam characters considered by that build.

The city now picks the closest intersecting character before checking cover. Street crowds reuse the spatial index for vehicle impacts, including the interpolated positions of distant residents. Seated people use a shorter hit volume. All seats share story targets and use the same shooting code during a mission and in free roam. The game tests cover a passing bystander intercepting a contract shot, a seated worker, a distant resident, two players, Space as an alternate trigger, and editing Luau while a shot and an authored contract are active. The game changes are in `e9c1d634`.

[Controls and game guide](../../../games/warp-theft-auto/README.md) · [Temporal reconstruction](../temporal-upscaling.md)
