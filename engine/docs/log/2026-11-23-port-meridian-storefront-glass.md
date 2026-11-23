# Port Meridian storefront reflections

The Civic Center office in seed 1948 has a broad storefront and an open door. At noon in the previous browser playtest (`34a8d24e`), an observer across the street could see very little difference between the glass and the open entrance. Both showed the same dark room with a small difference in tint.

In `c02a2bd3`, each visible glazed opening on a distant ground floor samples the environment along the mirror direction. The return includes the current sky, filtered markings from the street material and coarse matte facades in up to four 56 m lots. The boxes use the district layout and typical dimensions. The approximation omits traffic and pedestrians. A two-interface coating blends the reflection with the existing room view. The fixtures illuminate both views of the lobby, through the glass and through the open door.

On the RTX 5070 Ti native replay at 960 × 540, the standard deviation of pixel luminance within the same projected pane at 24 m and 62 degrees increased from 0.74 to 15.46 on a 0–255 scale. At the adjacent open doorway, mean luminance measured 14.04 before and 14.06 after. At 22:30 the lobby's desk and wall art were visible behind the reflections. Seven interior simulation regressions passed, including split-screen, collision, and room loading and unloading.

A geometric check used the left edge of the Ferry Exchange clock. At 14 m the predicted and observed reflected positions on the facade were -2.671 m and -2.668 m. At 24 m they were -2.479 m and -2.476 m. Both were within one pixel. In the fixed-camera native test, loading the office changed the sampled daylight pane by 0.25 and 0.32 mean absolute luminance at the two eye positions. Repeating the same 13 cm camera move with the room unloaded changed the pane by 1.40. A fixed camera at night measured 1.31 on arrival and 1.02 on departure.

In public WebGPU build `c02a2bd3-20261123065637`, 12 matched camera poses covered two distances, two times of day, and changes in angle and eye height. Two keyboard walks approached and left the office. Mean absolute luminance differences between moving frames spanning residency changes were 2.48 / 1.82 during the day and 2.10 / 1.66 at night. These moving-camera measurements include reflection parallax and foreground pedestrians. Holding the browser camera still and changing room residency produced a difference of 0.15 to 0.20.

## Frame times

Four alternating native runs (before, after, after, before) each recorded 120 warmed 960 × 540 frames per viewpoint on the RTX 5070 Ti. These are means of the two per-build medians. The final column measures CPU time waiting for GPU completion after submission.

| Office view | Frame wall before / after (ms) | Frame wall p95 before / after (ms) | GPU wait before / after (ms) |
| --- | ---: | ---: | ---: |
| Day, facing the glass | 3.75 / 4.58 | 4.96 / 5.76 | 1.83 / 1.87 |
| Day, oblique | 4.08 / 5.22 | 5.09 / 6.19 | 1.89 / 1.90 |
| Night, facing the glass | 3.11 / 4.16 | 4.36 / 5.20 | 1.15 / 1.19 |
| Night, oblique | 3.68 / 4.15 | 4.81 / 5.35 | 1.25 / 1.28 |
