# Port Meridian interiors and residents

Windows, NVIDIA GeForce RTX 5070 Ti, optimized `quest` build at `f68bb65d`, city seed 1948. The installed game rendered four independent local views. The room scenario put the players in two offices, a hotel and a warehouse. All four lighting volumes finished baking, using 0.49–0.84 seconds of worker time per room. Together the rooms and city occupied 975 of 1,024 world entities. The shared crowd selected 37 indoor residents across the four rooms, with 40 ambient slots available.

| Four-player scene | Output | Game CPU median | Render submission CPU median | GPU completion wait median | Frame wall median | Frame wall p95 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Four occupied interiors | 1920 × 1080 | 0.87 ms | 7.77 ms | 6.95 ms | 15.89 ms | 18.72 ms |
| Two pedestrians, one driver and one pilot | 3840 × 2160 | 0.95 ms | 6.43 ms | 9.26 ms | 16.99 ms | 19.43 ms |

Both samples contain 90 warmed frames at a fixed simulation tick using native headless rendering. The interior sample followed 520 rendered frames to allow all four asynchronous bakes to finish. The mixed outdoor sample followed its screenshot and 12 warmup frames. Times exclude startup, simulation and screenshot readback. The 4K frame time exceeds the engine’s 8.33 ms target for 120 Hz. Reproduce the room sample with the four-player scenario and set the outdoor scenario’s `size` to `[3840, 2160]` for the mixed sample. [Four interiors](../../../games/warp-theft-auto/scripts/four-interiors.json) · [Four outdoor views](../../../games/warp-theft-auto/scripts/four-seats.json)

The shared human generator’s 16,384-seed regression put 67.6% of heights within one standard deviation of the midpoint. At a fixed height, the percentages beyond two standard deviations of the mean were 4.3% for chest breadth, 4.5% for waist and 4.5% for hips. Correlations between chest and waist, waist and hips, and hips and chest were 0.81, 0.98 and 0.71. These measure the procedural generator. [Warp Human geometry regression](../../crates/render/src/humans/geometry.rs)
