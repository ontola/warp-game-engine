# Warp Theft Auto on the RTX 5070 Ti

Windows, NVIDIA GeForce RTX 5070 Ti, optimized `quest` Cargo profile. Port Meridian uses seed 1948, 27 planted pines from two cooked shapes, and four simultaneous local views. Two players are walking in separate districts, one is driving, and one is hovering above the harbor. The city, camera poses, HUD, vehicle physics and asset settings are recorded by the [four-seat scenario](../../../games/warp-theft-auto/scripts/four-seats.json).

| Full output | Game draw CPU median | Render submission CPU median | GPU completion wait median | Frame wall median | Frame wall p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1920 × 1080 | 0.68 ms | 3.89 ms | 3.94 ms | 8.76 ms | 9.73 ms |
| 3840 × 2160 | 0.78 ms | 4.84 ms | 8.22 ms | 14.10 ms | 15.62 ms |

Each run used the native headless render path with no resolution cap. The first screenshot and 12 warmup frames are excluded. The next 90 frames render the same tick and wait for GPU completion individually. The completion wait excludes work already overlapped with the CPU. Simulation, startup, screenshot readback and window presentation are outside these steady-frame numbers. The native 4K run exceeds the 8.33 ms target for 120 Hz.

The 1080p run used the game changes first committed as `fc5be598`. The 4K run used the integrated game revision `a5d01d1c`. To reproduce the 4K sample, make a copy of the four-seat scenario, set its `size` to `[3840, 2160]`, and run it with `--players 4`. The [game guide](../../../games/warp-theft-auto/README.md) gives the headless commands.
