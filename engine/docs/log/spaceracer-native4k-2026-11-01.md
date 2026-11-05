# SpaceRacer native 4K investigation — 1 November 2026

City reflections were the main GPU bottleneck. Replacing median-split reflection
bounds with a surface-area hierarchy and threading its traversal reduced the
same scene's median measured GPU interval by about **28%**, without removing
content or reducing resolution. The 3840×2160 reference image is pixel-identical.
The packaged build now defaults to release rather than development optimization.

This is a headless cost investigation, **not a claim of locked 4K/120 FPS**.
No interactive playtest was opened. Four-player city still exceeds the 8.33 ms
GPU budget before HUD/presentation. The former Godot runtime was not run.

## Setup and evidence

- Ryzen 9 5900X (12 cores / 24 logical processors), RTX 5070 Ti, NVIDIA 617.14,
  Vulkan. Connected output reports 3840×2160 at 120 Hz.
- All samples render native 3840×2160. No reconstruction, adaptive resolution,
  geometry omission or effect reduction. Motion blur remains enabled.
- `app::profile::bench` records CPU wall-time phases and asynchronous GPU
  timestamp intervals by frame ID. Warmup and pixel readback are excluded.
  Every measured run has complete timestamp coverage, zero drops/errors/pending.
- Raw local captures: `build/spaceracer-perf-native4k`,
  `build/spaceracer-perf-final4k`, `build/spaceracer-perf-final-biomes4k`.
  [Committed summaries and metadata](../measurements/spaceracer-native4k-2026-11-01.json)
  retain the distributions, configurations and background CPU sample.

The old `app::bench` returned throughput including a full 4K pixel readback
every frame. Its initial ~29 FPS result is **not gameplay FPS**. SpaceRacer now
uses the new no-readback profile; legacy callers retain their documented API.

## Matched reflection comparison

City, seed 35, hard, one player, demo drivers; advance 1200 ticks, warm up 60
frames, measure 240 frames at 120 simulation ticks/second. Both binaries use
the dev build with identical diagnostic environment: `WGPU_DEBUG=0`,
`WGPU_VALIDATION=0`, `VK_LOADER_LAYERS_DISABLE=~implicit~`. No physics, AI, asset,
light, shader appearance, geometry, blur, bloom or sampling setting changed.
CPU measurements are kept separate from GPU time.

| Run | GPU interval p50 / p95 | Surface shading p50 |
| --- | ---: | ---: |
| Before, first run | 8.588 / 10.013 ms | 5.522 ms |
| After, first run | 6.179 / 7.032 ms | 3.146 ms |
| Before, repeated | 8.564 / 9.969 ms | See JSON |
| After, repeated | 6.139 / 7.213 ms | See JSON |

The diagnostic omission `--profile-no-reflections` isolated roughly 3.6 ms of
the original shading cost in an earlier run. That omission is **not shipped as
a quality change**. The actual optimization retains all facade boxes and
billboard planes, changes how they are searched, removes the private per-pixel
stack and repeated child tests, and removes the old 256-visit truncation.
Traversal is bounded by the generated node count and strictly advancing links.

The build happens once during level generation. Tests check all proxy bounds,
10,000 coincident proxies, bounded builder depth and threaded nearest hits
against exhaustive intersection for 200 rays through 900 proxies.
Before/after native screenshots at tick 1320 have zero RGBA pixel difference.
This checks one representative view, not every possible seed and reflection.

## Final optimized build

Separate later-race coverage: release build, hard, demo, tick 2400, 120 warmup
frames and 360 measured frames per case. Values are **GPU interval p50 / p95**,
not FPS. These positions differ from the matched comparison above.

| Biome, seed 35 | One player | Two players | Four players |
| --- | ---: | ---: | ---: |
| City | 7.74 / 8.88 ms | 8.24 / 9.24 ms | 8.96 / 9.73 ms |
| Forest | 5.72 / 6.26 ms | 6.01 / 6.72 ms | Not sampled |
| Cell | 5.34 / 5.66 ms | 5.45 / 5.78 ms | Not sampled |
| Desert | 4.23 / 4.71 ms | 4.43 / 4.71 ms | Not sampled |
| Ocean | 4.14 / 4.52 ms | 4.30 / 4.60 ms | Not sampled |

City seed 80385 also measured 8.22/8.78 ms (one player), 7.90/8.39 ms (two),
and 8.91/9.48 ms (four). CPU submission in city seed 35 was 2.41/4.58 ms,
3.35/5.95 ms and 5.88/9.84 ms respectively. In four views, driver submission
alone accounts for 2.91/5.70 ms; simulation is 0.65/1.81 ms. Keep gameplay in
Luau: moving it wholesale into Rust would not address the main measured costs.

The native GPU work is comfortably below a 16.67 ms budget in these samples,
but 120 FPS is not assured, particularly in city and four-view CPU tails.
GPU timestamps exclude HUD/presentation; CPU and GPU overlap. A serial
headless completion wait is not presented-frame latency. Frame pacing,
controller latency, cold shader hitches and maximum simultaneous explosions
need separate tests. The user explicitly requested no playtest afterwards.

## Background contention

Background programs remained running. With no game/build active, a three-second
CPU delta sample showed approximately 4.2 cores used by Windows' Now Playing
Session Manager (`NPSMSvc_1fb17c3`), 1.3 by GameNight's daemon, 1.2 by
CrossDeviceService, 1.7 across two Spotify processes and 0.5 by the browser.
Here 100% means one logical core, not the whole machine. This is material
contention and can contribute to CPU frame-time tails. No unrelated process or
service was stopped. These snapshots do not establish the cause of every hitch.

## Tooling and next work

[Native profiling](../native-profiling.md) describes the shared API, serial matrix
runner and offline HTML report viewer. Unlike the old throughput figure, it
records raw per-frame CPU phases, GPU pass/view intervals, percentiles, dropped
samples, resolution/reconstruction and reproducible scenario/build information.
The viewer was rendered headlessly at desktop and phone sizes; raw JSON remains
the source of the numerical claims.

The next useful engine work is live presented-frame tracing with fixed-tick
catch-up counts, driver/acquire/present spans, Luau hook/GC timing, HUD GPU scopes
and opt-in background telemetry. Then use that evidence to reduce four-view
submission overhead and the remaining city shading cost. Do not respond to CPU
contention merely by lowering rendering resolution.
